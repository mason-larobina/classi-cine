//! ffprobe integration.
//!
//! The cache module is decoupled from the actual probing via the [`Probe`]
//! trait so the cache logic can be tested with a stub and the real backend
//! swapped later. [`FfprobeProbe`] is the production impl that shells out to
//! the `ffprobe` binary.

use crate::Error;
use crate::cache::MediaFeatures;
use crate::walk::File as WalkFile;

/// Probe a single file for its extracted media features.
///
/// Takes a [`walk::File`](crate::walk::File) so the impl can source
/// `file_size` from the already-collected stat rather than re-statting or
/// asking ffprobe for it. Returns an error if ffprobe fails or its output is
/// unusable (no video stream, no duration, unparseable JSON).
pub trait Probe {
    fn probe(&self, file: &WalkFile) -> Result<MediaFeatures, Error>;
}

/// `Probe` implementation that shells out to the `ffprobe` binary.
pub struct FfprobeProbe;

impl FfprobeProbe {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FfprobeProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe for FfprobeProbe {
    fn probe(&self, file: &WalkFile) -> Result<MediaFeatures, Error> {
        let output = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
            ])
            .arg(file.path.as_ref())
            .output()?;

        if !output.status.success() {
            return Err(Error::ProbeFailed {
                path: file.path.to_string_lossy().into(),
                reason: String::from_utf8_lossy(&output.stderr).into(),
            });
        }

        let json: FfprobeJson =
            serde_json::from_slice(&output.stdout).map_err(|e| Error::ProbeFailed {
                path: file.path.to_string_lossy().into(),
                reason: format!("unparseable ffprobe JSON: {}", e),
            })?;

        MediaFeatures::from_ffprobe(&json, file.size).map_err(|reason| Error::ProbeFailed {
            path: file.path.to_string_lossy().into(),
            reason,
        })
    }
}

// ---------------------------------------------------------------------------
// ffprobe JSON schema (only the fields we extract)
// ---------------------------------------------------------------------------

/// Top-level ffprobe JSON output: a `format` object and a `streams` array.
#[derive(Debug, serde::Deserialize)]
pub struct FfprobeJson {
    #[serde(default)]
    pub format: Option<FfprobeFormat>,
    #[serde(default)]
    pub streams: Vec<FfprobeStream>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FfprobeFormat {
    /// Duration in seconds, as a decimal string (e.g. `"7200.500000"`).
    #[serde(default)]
    pub duration: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct FfprobeStream {
    #[serde(default, rename = "codec_type")]
    pub codec_type: Option<String>,
    #[serde(default, rename = "codec_name")]
    pub codec_name: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Average frame rate as a `"num/den"` string (e.g. `"24000/1001"`).
    #[serde(default, rename = "avg_frame_rate")]
    pub avg_frame_rate: Option<String>,
}

impl MediaFeatures {
    /// Extract features from parsed ffprobe JSON. `file_size` is sourced from
    /// `walk::File::size` (passed in), not from ffprobe.
    ///
    /// Returns `Err(String)` (a probe-failure reason) when the output is
    /// unusable: no video stream, missing width/height, or missing duration.
    /// Unreliable optional fields (codec names, fps) become `None`/empty
    /// rather than failing the whole probe.
    pub fn from_ffprobe(json: &FfprobeJson, file_size: u64) -> Result<Self, String> {
        let video = json
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
            .ok_or_else(|| "no video stream".to_string())?;

        let width = video
            .width
            .ok_or_else(|| "video stream missing width".to_string())?;
        let height = video
            .height
            .ok_or_else(|| "video stream missing height".to_string())?;

        let video_codec = video.codec_name.clone().unwrap_or_default();

        let audio_codec = json
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("audio"))
            .and_then(|s| s.codec_name.clone())
            .unwrap_or_default();

        // Duration is required: ffprobe reports it for any playable file.
        let duration_secs = json
            .format
            .as_ref()
            .and_then(|f| f.duration.as_deref())
            .ok_or_else(|| "missing format.duration".to_string())
            .and_then(|d| {
                d.parse::<f64>()
                    .map_err(|e| format!("unparseable duration {:?}: {}", d, e))
            })?;

        let fps = video.avg_frame_rate.as_deref().and_then(eval_frame_rate);

        Ok(MediaFeatures {
            width,
            height,
            file_size,
            video_codec,
            audio_codec,
            duration_secs,
            fps,
        })
    }
}

/// Evaluate an ffprobe `"num/den"` frame-rate string to a float. Returns
/// `None` for missing, malformed, or zero-denominator values.
fn eval_frame_rate(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 {
        return None;
    }
    Some(num / den)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(
        codec_type: &str,
        codec_name: Option<&str>,
        w: Option<u32>,
        h: Option<u32>,
        fps: Option<&str>,
    ) -> FfprobeStream {
        FfprobeStream {
            codec_type: Some(codec_type.into()),
            codec_name: codec_name.map(String::from),
            width: w,
            height: h,
            avg_frame_rate: fps.map(String::from),
        }
    }

    fn format(duration: Option<&str>) -> FfprobeFormat {
        FfprobeFormat {
            duration: duration.map(String::from),
        }
    }

    #[test]
    fn from_ffprobe_extracts_all_fields() {
        let json = FfprobeJson {
            format: Some(format(Some("7200.500000"))),
            streams: vec![
                stream(
                    "video",
                    Some("h264"),
                    Some(1920),
                    Some(1080),
                    Some("24000/1001"),
                ),
                stream("audio", Some("ac3"), None, None, None),
            ],
        };
        let f = MediaFeatures::from_ffprobe(&json, 8_589_934_592).unwrap();
        assert_eq!(f.width, 1920);
        assert_eq!(f.height, 1080);
        assert_eq!(f.file_size, 8_589_934_592);
        assert_eq!(f.video_codec, "h264");
        assert_eq!(f.audio_codec, "ac3");
        assert!((f.duration_secs - 7200.5).abs() < 1e-9);
        assert!((f.fps.unwrap() - 23.976023).abs() < 1e-3);
    }

    #[test]
    fn from_ffprobe_defaults_optional_fields() {
        // No codec names, no fps, but valid video + duration.
        let json = FfprobeJson {
            format: Some(format(Some("90.0"))),
            streams: vec![stream("video", None, Some(1280), Some(720), None)],
        };
        let f = MediaFeatures::from_ffprobe(&json, 100).unwrap();
        assert_eq!(f.video_codec, "");
        assert_eq!(f.audio_codec, "");
        assert_eq!(f.fps, None);
        assert!((f.duration_secs - 90.0).abs() < 1e-9);
    }

    #[test]
    fn from_ffprobe_errors_without_video_stream() {
        let json = FfprobeJson {
            format: Some(format(Some("90.0"))),
            streams: vec![stream("audio", Some("aac"), None, None, None)],
        };
        assert!(MediaFeatures::from_ffprobe(&json, 100).is_err());
    }

    #[test]
    fn from_ffprobe_errors_without_duration() {
        let json = FfprobeJson {
            format: Some(format(None)),
            streams: vec![stream("video", Some("h264"), Some(1920), Some(1080), None)],
        };
        assert!(MediaFeatures::from_ffprobe(&json, 100).is_err());
    }

    #[test]
    fn from_ffprobe_errors_without_dimensions() {
        let json = FfprobeJson {
            format: Some(format(Some("90.0"))),
            streams: vec![stream("video", Some("h264"), None, None, None)],
        };
        assert!(MediaFeatures::from_ffprobe(&json, 100).is_err());
    }

    #[test]
    fn eval_frame_rate_handles_forms() {
        assert!((eval_frame_rate("24000/1001").unwrap() - 23.976).abs() < 1e-2);
        assert!((eval_frame_rate("25/1").unwrap() - 25.0).abs() < 1e-9);
        assert!(eval_frame_rate("0/0").is_none());
        assert!(eval_frame_rate("garbage").is_none());
        assert!(eval_frame_rate("30").is_none()); // missing slash
    }
}
