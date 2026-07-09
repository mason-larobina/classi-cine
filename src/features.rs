//! Transformation of raw [`MediaFeatures`] (from the ffprobe cache) into
//! classifier feature tokens. See `docs/media-features-classifier.md`.
//!
//! All transformation is **in-memory and read-time**: nothing here is
//! persisted. The cache stores raw values; this module is the discretization
//! layer that lives entirely inside the classifier, so the bucketing /
//! smoothing strategy stays mutable without re-probing the library (the
//! raw-on-disk principle from `docs/ffprobe-cache.md`).
//!
//! Feature tokens are minted from the **same `TokenMap` the path
//! `PairTokenizer` already owns**, so their ids coexist safely with path ids
//! in the merged `Ngrams` vec. They are *used as-is*: they never enter the
//! `PairTokenizer` and are never byte-pair-merged. They are expanded only via
//! orderless [`Ngrams::combinations`](crate::ngrams::Ngrams::combinations),
//! never path `windows`.

use crate::cache::MediaFeatures;
use crate::tokens::{Token, TokenMap, Tokens};

/// Tunable parameters for feature token generation, captured from
/// [`CommonArgs`](crate::CommonArgs) and passed through the pipeline so the
/// pure functions here stay free of I/O and classifier state.
#[derive(Debug, Clone, Copy)]
pub struct FeatureConfig {
    /// Geometric bucket base for `duration` / `filesize` / `bitrate` (> 1.0).
    pub bucket_base: f64,
    /// Geometric bucket base for `fps` (> 1.0). Finer than `bucket_base`
    /// because the fps range is narrow and clustered at standard rates.
    pub fps_base: f64,
    /// Neighbor smoothing half-width. `0` = plain 1-bucket singletons (no
    /// neighbor coupling). See `docs/media-features-classifier.md` *Smoothing*.
    pub smoothing: usize,
    /// Orderless cross-feature combination order. `0` disables feature
    /// ngrams entirely (the feature `combinations` call is skipped). `1`
    /// emits only singletons; `2` (default) emits singletons + pairs.
    pub combinations: usize,
}

impl FeatureConfig {
    /// Build a config from [`CommonArgs`](crate::CommonArgs), asserting the
    /// bucket bases are `> 1.0` (geometric bucketing is undefined at `<= 1.0`).
    pub fn from_common(common: &crate::CommonArgs) -> Self {
        assert!(
            common.features_bucket_base > 1.0,
            "--features-bucket-base must be > 1.0"
        );
        assert!(
            common.features_fps_base > 1.0,
            "--features-fps-base must be > 1.0"
        );
        Self {
            bucket_base: common.features_bucket_base,
            fps_base: common.features_fps_base,
            smoothing: common.features_smoothing,
            combinations: common.features_combinations,
        }
    }
}

/// Geometric bucket index: `floor(log_base(max(v, 1.0)))`.
///
/// Returns `None` for `v < 1.0` (the `max(v, 1.0)` clamp forces `i >= 0`, so
/// sub-1 values collapse to bucket 0; we instead emit **no token** for them,
/// which is the desired "no data" behavior for features like a zero duration)
/// or non-finite / non-positive values.
fn bucket(v: f64, base: f64) -> Option<i64> {
    if !v.is_finite() || v < 1.0 {
        return None;
    }
    let i = (v.ln() / base.ln()).floor() as i64;
    // Defensive: with v >= 1.0 and base > 1.0, i is always >= 0. The clamp
    // keeps the contract explicit.
    if i < 0 { None } else { Some(i) }
}

/// Reduce a `w:h` ratio by its GCD. Returns `None` if either dimension is zero
/// (no / failed probe → no aspect token), honoring the "neutral when no
/// features" guarantee: a default `MediaFeatures` emits an empty feature
/// vector.
fn reduce_ratio(w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let g = gcd(w, h);
    Some((w / g, h / g))
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Push neighbor-singleton token **strings** `name:k` for every `k` in
/// `[i-w, i+w]` (clamped to `>= 0`), where `i = bucket(v, base)`. `w == 0`
/// yields just `name:i`. Indices are always distinct integers (the lower
/// clamp never duplicates an upper index), so no explicit dedup is needed.
///
/// This is the string core of the smoothing scheme: a value in bucket `i`
/// also emits its immediate neighbors, so adjacent buckets share signal
/// through overlapping singletons and the trained count of each `s_k` becomes
/// a `(2w+1)`-bucket boxcar sum of the raw histogram — a low-variance
/// estimate even when individual buckets hold only one or two files. The
/// token-id form ([`feature_tokens`]) simply mints each string into the
/// shared [`TokenMap`].
fn emit_neighbor_strings(out: &mut Vec<String>, name: &str, v: f64, base: f64, w: usize) {
    let Some(i) = bucket(v, base) else {
        return;
    };
    let lo = (i - w as i64).max(0);
    let hi = i + w as i64;
    for k in lo..=hi {
        out.push(format!("{}:{}", name, k));
    }
}

/// Evidence that ffprobe actually reported something for this file (as
/// opposed to an all-zero `MediaFeatures::default()` left behind by a probe
/// failure). `file_size` is intentionally excluded: it comes from the walk,
/// not ffprobe, so a failed probe still has `file_size > 0`.
///
/// This gates the `audio_codec:none` token so that a failed probe produces a
/// truly empty feature vector — the "neutral when no features" guarantee from
/// `docs/media-features-classifier.md`. A real video-only file (which has a
/// video stream and duration but no audio) still emits `audio_codec:none` as a
/// genuine signal.
pub(crate) fn has_probe_data(f: &MediaFeatures) -> bool {
    !f.video_codec.is_empty()
        || f.duration_secs > 0.0
        || (f.width > 0 && f.height > 0)
        || f.fps.is_some()
}

/// Build the raw feature token **strings** (without `root`/`eol` sentinels)
/// for one file's [`MediaFeatures`]. Pure: it does not touch any
/// [`TokenMap`], so it is safe to call from the TUI render path without
/// mutating the shared map or minting spurious ids.
///
/// Categorical / derived features emit one token per unique value;
/// continuous, near-unique features (`duration`, `filesize`, `bitrate`,
/// `fps`) are bucketed geometrically and expanded into neighbor singletons.
/// See `docs/media-features-classifier.md` *Feature vocabulary*.
pub fn feature_token_strings(f: &MediaFeatures, cfg: &FeatureConfig) -> Vec<String> {
    let mut out = Vec::new();

    // --- Categorical / derived ---
    if !f.video_codec.is_empty() {
        out.push(format!("video_codec:{}", f.video_codec));
    }
    if f.audio_codec.is_empty() {
        // "No audio stream" is a real signal — but only when the probe
        // actually succeeded. A failed probe (all-default `MediaFeatures`)
        // has an empty `audio_codec` too; emitting `audio_codec:none` there
        // would violate the neutrality guarantee, so gate on `has_probe_data`.
        if has_probe_data(f) {
            out.push("audio_codec:none".to_string());
        }
    } else {
        out.push(format!("audio_codec:{}", f.audio_codec));
    }
    if let Some((aw, ah)) = reduce_ratio(f.width, f.height) {
        out.push(format!("aspect:{}:{}", aw, ah));
    }
    if f.width > 0 && f.height > 0 {
        out.push(format!("resolution:{}x{}", f.width, f.height));
    }

    // --- Continuous: bucket + neighbor singletons ---
    // fps uses its own finer base (default 1.1); the others share bucket_base.
    if let Some(fps) = f.fps {
        emit_neighbor_strings(&mut out, "fps", fps, cfg.fps_base, cfg.smoothing);
    }
    if f.duration_secs > 0.0 {
        emit_neighbor_strings(
            &mut out,
            "duration",
            f.duration_secs,
            cfg.bucket_base,
            cfg.smoothing,
        );
    }
    if f.file_size > 0 {
        emit_neighbor_strings(
            &mut out,
            "filesize",
            f.file_size as f64,
            cfg.bucket_base,
            cfg.smoothing,
        );
    }
    if f.duration_secs > 0.0 {
        let bitrate = (f.file_size as f64) * 8.0 / f.duration_secs;
        if bitrate.is_finite() && bitrate > 0.0 {
            emit_neighbor_strings(&mut out, "bitrate", bitrate, cfg.bucket_base, cfg.smoothing);
        }
    }
    out
}

/// Build the raw feature token vector (without `root`/`eol` sentinels) for one
/// file's [`MediaFeatures`], minting each token string into the shared
/// [`TokenMap`].
///
/// Thin wrapper over [`feature_token_strings`]: the bucketing / smoothing
/// logic lives there (string-valued, pure) and is shared with the TUI render
/// path; this just resolves each string to its stable [`Token`] id in the
/// shared map.
pub fn feature_tokens(f: &MediaFeatures, map: &mut TokenMap, cfg: &FeatureConfig) -> Vec<Token> {
    feature_token_strings(f, cfg)
        .into_iter()
        .map(|s| map.get_or_create_token(&s))
        .collect()
}

/// Wrap a feature token vector in the `root` / `eol` sentinels as a [`Tokens`]
/// whose bloom is left empty.
///
/// Feature tokens are only ever consumed by [`Ngrams::combinations`], which is
/// orderless and bloom-independent (it reads `tokens.as_slice()` and filters
/// out specials via `last_special`). The bloom-dependent `contains` / `pairs`
/// methods must not be called on the result; use
/// [`Tokens::from_token_vec_unchecked`](crate::tokens::Tokens::from_token_vec_unchecked)
/// semantics (which this wraps).
pub fn build_feature_tokens(f: &MediaFeatures, map: &mut TokenMap, cfg: &FeatureConfig) -> Tokens {
    let mut tv = Vec::new();
    tv.push(map.root());
    tv.extend(feature_tokens(f, map, cfg));
    tv.push(map.eol());
    Tokens::from_token_vec_unchecked(tv)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn map() -> TokenMap {
        TokenMap::new("")
    }

    fn cfg() -> FeatureConfig {
        FeatureConfig {
            bucket_base: 1.5,
            fps_base: 1.1,
            smoothing: 1,
            combinations: 2,
        }
    }

    fn token_strs(tokens: &[Token], map: &TokenMap) -> Vec<String> {
        tokens
            .iter()
            .map(|t| map.get_str(*t).unwrap().to_string())
            .collect()
    }

    #[test]
    fn bucket_is_geometric_index() {
        // bucket(v) = floor(log_base(max(v,1.0)))
        assert_eq!(bucket(1.0, 1.5), Some(0));
        assert_eq!(bucket(1.5, 1.5), Some(1));
        assert_eq!(bucket(2.25, 1.5), Some(2)); // 1.5^2
        assert_eq!(bucket(1.49, 1.5), Some(0));
        // v < 1.0 -> None (no token).
        assert_eq!(bucket(0.0, 1.5), None);
        assert_eq!(bucket(0.999, 1.5), None);
        assert_eq!(bucket(-5.0, 1.5), None);
        assert_eq!(bucket(f64::NAN, 1.5), None);
        assert_eq!(bucket(f64::INFINITY, 1.5), None);
    }

    #[test]
    fn reduce_ratio_by_gcd() {
        assert_eq!(reduce_ratio(1920, 1080), Some((16, 9)));
        assert_eq!(reduce_ratio(640, 480), Some((4, 3)));
        assert_eq!(reduce_ratio(1280, 720), Some((16, 9)));
        assert_eq!(reduce_ratio(7, 7), Some((1, 1)));
        // Zero dimension -> None (no aspect token).
        assert_eq!(reduce_ratio(0, 1080), None);
        assert_eq!(reduce_ratio(1920, 0), None);
        assert_eq!(reduce_ratio(0, 0), None);
    }

    #[test]
    fn fps_standard_rates_coalesce() {
        // At base 1.1, 23.976 / 24 / 25 fall in the same bucket and thus emit
        // the same neighbor singleton set. Verified against the table in the
        // design doc.
        let c = cfg();
        let set_for = |fps: f64| {
            let mut out = Vec::new();
            emit_neighbor_strings(&mut out, "fps", fps, c.fps_base, c.smoothing);
            out
        };
        let s23976 = set_for(23.976);
        let s24 = set_for(24.0);
        let s25 = set_for(25.0);
        assert_eq!(s23976, s24);
        assert_eq!(s24, s25, "23.976/24/25 must coalesce at base 1.1");
        // Three tokens at w=1.
        assert_eq!(s23976.len(), 3);

        // 59.94 and 60 coalesce.
        assert_eq!(set_for(59.94), set_for(60.0));
    }

    #[test]
    fn emit_neighbors_width_zero_is_singleton() {
        let mut out = Vec::new();
        emit_neighbor_strings(&mut out, "duration", 100.0, 1.5, 0);
        assert_eq!(out.len(), 1, "w=0 emits exactly the bucket singleton");
    }

    #[test]
    fn emit_neighbors_lower_edge_clamps_to_zero() {
        // A value in bucket 0 with w=1 emits {s_0, s_1} (two tokens), never a
        // phantom s_{-1}.
        let mut out = Vec::new();
        emit_neighbor_strings(&mut out, "duration", 1.0, 1.5, 1);
        assert_eq!(
            out,
            vec!["duration:0".to_string(), "duration:1".to_string()]
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn default_features_emit_no_tokens() {
        // A probe failure (all-zero / empty MediaFeatures) must produce an
        // empty feature vector so missing features never distort ranking.
        let m = &mut map();
        let f = MediaFeatures::default();
        let out = feature_tokens(&f, m, &cfg());
        assert!(out.is_empty(), "default MediaFeatures must yield no tokens");
    }

    #[test]
    fn categorical_tokens_emitted() {
        let m = &mut map();
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 1_000_000,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_secs: 0.0, // avoid duration/bitrate tokens
            fps: None,
        };
        let out = feature_tokens(&f, m, &cfg());
        let strs = token_strs(&out, m);
        assert!(strs.contains(&"video_codec:h264".to_string()));
        assert!(strs.contains(&"audio_codec:aac".to_string()));
        assert!(strs.contains(&"aspect:16:9".to_string()));
        assert!(strs.contains(&"resolution:1920x1080".to_string()));
        // duration_secs == 0 -> no duration / bitrate tokens.
        assert!(!strs.iter().any(|s| s.starts_with("duration:")));
        assert!(!strs.iter().any(|s| s.starts_with("bitrate:")));
    }

    #[test]
    fn missing_audio_emits_none_token() {
        let m = &mut map();
        let f = MediaFeatures {
            width: 1280,
            height: 720,
            file_size: 1,
            video_codec: "hevc".into(),
            audio_codec: String::new(),
            duration_secs: 0.0,
            fps: None,
        };
        let out = feature_tokens(&f, m, &cfg());
        let strs = token_strs(&out, m);
        assert!(strs.contains(&"audio_codec:none".to_string()));
    }

    #[test]
    fn empty_video_codec_emits_no_token() {
        let m = &mut map();
        let f = MediaFeatures {
            width: 0,
            height: 0,
            file_size: 1,
            video_codec: String::new(),
            audio_codec: "aac".into(),
            duration_secs: 0.0,
            fps: None,
        };
        let out = feature_tokens(&f, m, &cfg());
        let strs = token_strs(&out, m);
        assert!(!strs.iter().any(|s| s.starts_with("video_codec:")));
    }

    #[test]
    fn continuous_features_emit_neighbor_triplets() {
        let m = &mut map();
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 8_000_000_000, // 8 GB
            video_codec: "h264".into(),
            audio_codec: "ac3".into(),
            duration_secs: 5400.0, // 90 min
            fps: Some(24.0),
        };
        let out = feature_tokens(&f, m, &cfg());
        let strs = token_strs(&out, m);
        // fps, duration, filesize, bitrate each contribute 3 tokens (w=1).
        let fps_n = strs.iter().filter(|s| s.starts_with("fps:")).count();
        let dur_n = strs.iter().filter(|s| s.starts_with("duration:")).count();
        let size_n = strs.iter().filter(|s| s.starts_with("filesize:")).count();
        let br_n = strs.iter().filter(|s| s.starts_with("bitrate:")).count();
        assert_eq!(fps_n, 3);
        assert_eq!(dur_n, 3);
        assert_eq!(size_n, 3);
        assert_eq!(br_n, 3);
    }

    #[test]
    fn build_feature_tokens_wraps_with_sentinels() {
        let m = &mut map();
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 1_000,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_secs: 0.0,
            fps: None,
        };
        let ft = build_feature_tokens(&f, m, &cfg());
        let slice = ft.as_slice();
        // First and last are the root/eol sentinels; middle are feature tokens.
        assert_eq!(slice.first().copied().unwrap(), m.root());
        assert_eq!(slice.last().copied().unwrap(), m.eol());
        assert!(slice.len() >= 3);
    }

    #[test]
    fn feature_tokens_share_map_with_paths() {
        // Minting the same feature string twice must return the same Token id
        // (shared map dedup), which is what makes merged ngrams safe: a
        // feature token and a path token only share an id when they are
        // literally the same string.
        let m = &mut map();
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 1_000,
            video_codec: "h264".into(),
            audio_codec: "aac".into(),
            duration_secs: 0.0,
            fps: None,
        };
        let a = feature_tokens(&f, m, &cfg());
        let b = feature_tokens(&f, m, &cfg());
        assert_eq!(a, b);
        // The video_codec token id equals a direct mint of the same string.
        let direct = m.get_or_create_token("video_codec:h264");
        assert!(a.iter().any(|t| *t == direct));
    }

    #[test]
    fn feature_token_strings_is_pure_and_matches_feature_tokens() {
        // `feature_token_strings` must not touch the TokenMap: calling it on a
        // fresh map leaves the map's count unchanged, and its output exactly
        // matches the strings the minting path (`feature_tokens`) would
        // resolve. This is what lets the TUI render feature tokens without
        // mutating the shared map.
        let m = &mut map();
        let before = m.count();
        let f = MediaFeatures {
            width: 1920,
            height: 1080,
            file_size: 8_000_000_000,
            video_codec: "h264".into(),
            audio_codec: "ac3".into(),
            duration_secs: 5400.0,
            fps: Some(24.0),
        };
        let strs = feature_token_strings(&f, &cfg());
        assert_eq!(m.count(), before, "pure path must not mint tokens");
        assert!(!strs.is_empty());

        // The minting path resolves exactly these strings, in order.
        let tokens = feature_tokens(&f, m, &cfg());
        let resolved: Vec<String> = tokens
            .iter()
            .map(|t| m.get_str(*t).unwrap().to_string())
            .collect();
        assert_eq!(resolved, strs);
    }
}
