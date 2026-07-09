# MediaFeatures Tokenization & Classifier — Design

This doc designs how `MediaFeatures` (the raw ffprobe values defined in
[`ffprobe-cache.md`](./ffprobe-cache.md)) are turned into classifier features.
The cache stores **raw** values; this doc is the read-time transformation layer
that lives entirely in memory inside the classifier — bucketing, smoothing, and
discretization are **never persisted**, so the discretization strategy stays
mutable without re-probing the library (the raw-on-disk principle from
`ffprobe-cache.md`).

## Goals

- **Reuse the Naive Bayes substrate.** Feature tokens feed the same
  `Ngrams` / `NaiveBayesClassifier` machinery the path classifier already
  uses; no new statistical model and **no second classifier instance**.
  Feature ngrams are appended into the same per-entry `Ngrams` vec (and thus
  the same ngram vocabulary) as the path ngrams, deduplicated together, and
  trained/scored by the single existing `NaiveBayesClassifier`. This works
  precisely because the classifier is Bernoulli (presence/absence only) and
  `Ngrams` dedups its vec — see *Why merge into the path NB*.
- **Keep BPE pure.** Feature tokens are *used as-is* — they never enter the
  `PairTokenizer` and are never byte-pair-merged. They are minted from the
  shared `TokenMap` (so their ids coexist safely with path ids) but built in
  their own `Vec<Token>` and expanded only via orderless `combinations`, never
  path `windows`.
- **Categorical features → one token per unique value.** `video_codec`,
  `audio_codec`, derived `aspect_ratio`, and discretized `fps` become stable
  string tokens like `video_codec:av1`, `aspect:16:9`.
- **Continuous, near-unique features → bucketed.** `duration`, `file_size`,
  and derived `bitrate` are too sparse to use raw. They are bucketed
  geometrically (default base **1.5**, not power-of-2 — see *Bucketing*).
- **Smooth across buckets.** A value in bucket `i` should also "boost or lower
  nearby buckets": adjacent buckets share signal so the classifier treats
  near-equal values as similar rather than unrelated. Done via
  *neighbor-singleton tokens* (see *Smoothing*): a value in bucket `i` also
  emits its immediate neighbors (`bucket:(i-1)` + `bucket:i` + `bucket:(i+1)`),
  so adjacent buckets share signal through their overlapping singletons.
- **Orderless by nature.** Features have no inherent order, so only
  **combinations** (orderless) are generated for feature tokens — never
  **windows** (contiguous). Controlled independently from the path
  `--combinations` via `--features-combinations`.

## Non-goals

- No change to the ffprobe cache format or the raw `MediaFeatures` schema.
- No new statistical model; features reuse the existing
  `NaiveBayesClassifier` instance (no second instance, no extra score column).
- No persistence of buckets, intervals, or any derived value.
- No learning of feature weights / no calibration of the feature column beyond
  the existing per-column min-max normalization in
  `App::calculate_scores_and_sort_entries`.

## Relationship to existing code

- **`cache::MediaFeatures`** (`src/cache.rs`) is the raw input. `width`,
  `height`, `file_size`, `video_codec`, `audio_codec`, `duration_secs`, `fps`
  are all already present and raw. Derived `aspect_ratio` (reduce `w:h` by GCD)
  and `bitrate` (`file_size * 8 / duration_secs`) are computed at read time, as
  `ffprobe-cache.md` already mandates.
- **`ngrams::Ngrams`** (`src/ngrams.rs`): `Ngrams::combinations(tokens, k,
  last_special, allowed, debug)` already generates orderless, deduplicated
  combinations of non-special tokens, sorted before hashing so `{a,b}` and
  `{b,a}` collapse. Crucially, `combinations` **appends** to the vec (it does
  not clear it, unlike `windows`) and finishes with a sort-and-dedup, so a
  second `combinations` call on the feature tokens merges its output into the
  same deduplicated vec as the path windows/combinations. Feature ngrams reuse
  this verbatim — no new ngram code.
- **`tokens::{Token, TokenMap, Tokens}`** (`src/tokens.rs`): `TokenMap` mints
  stable `Token(u32)` ids from strings via `get_or_create_token`. Feature
  tokens are minted from the **same `TokenMap` the path `PairTokenizer`
  already owns** — not a separate map. `Token` is an opaque `u32` id with no
  map identity, so ids minted in a separate map would be indistinguishable
  from path ids and could cause false ngram collisions; sharing the map avoids
  this and lets feature ngrams coexist safely in the same `Ngrams` vec.
  `PairTokenizer` is still never *applied* to feature strings (they are never
  split into characters or BPE-merged); the map is only used for
  `get_or_create_token` on whole feature strings.
- **`classifier::NaiveBayesClassifier`** (`src/classifier.rs`): trained with
  `train_positive`/`train_negative(&Ngrams)`, scored with
  `calculate_score(&Entry)`. It is **Bernoulli** — `train_*` iterates the
  (deduplicated) `Ngrams` vec and adds 1 per *unique* ngram, and
  `calculate_score` sums log-likelihoods over *present* ngrams, so frequency
  within a document never matters. Therefore the same single instance can
  consume a merged vec of path + feature ngrams with no double-counting: the
  only new code is what builds the feature tokens appended into that vec.
- **`app::Entry`** (`src/app.rs`): already carries `tokens`, `ngrams`,
  `scores: Box<[f64]>`, and `features: MediaFeatures`. No new field and no
  new score column — feature ngrams are appended into the existing
  `entry.ngrams`, and the existing `naive_bayes` column already scores them.
- **`App::get_classifiers`** returns `Vec<&dyn Classifier>`; unchanged. The
  single `naive_bayes` classifier now scores path + feature ngrams together,
  so the column set stays naive_bayes / file_size / dir_size / file_age.

## Feature vocabulary

Feature tokens are minted from the **same `TokenMap` the path `PairTokenizer`
already owns** (`tokenizer.token_map()`), not a separate map. There is no
wrapper type — the feature pipeline calls `TokenMap::get_or_create_token` and
`TokenMap::last_special` / `root` / `eol` directly on the tokenizer's map. The
map is *never* passed to a second `PairTokenizer::new`, and feature strings are
never fed to `PairTokenizer::tokenize`, so they are never split into characters
or BPE-merged — the map is used only for `get_or_create_token` on whole
feature strings.

Sharing the map is what makes merging safe: `Token` is an opaque `u32` with no
map identity, so ids minted in a *separate* map would be indistinguishable
from path ids and could collide when hashed into `Ngram`s. With one shared
map, a feature token and a path token only share an id when they are literally
the same string (vanishingly unlikely given the `video_codec:` / `duration:`
prefixes), and even then the dedup in `Ngrams` collapses them to a single
Bernoulli feature — correct behavior. All feature token ids land above
`last_special`, so `Ngrams::combinations`'s special-token filter admits them
and excludes only the root/eol/unk sentinels.

### Token strings

Categorical / derived features emit **one token per unique value**:

| Feature           | Token format                      | Notes |
|-------------------|-----------------------------------|-------|
| `video_codec`      | `video_codec:h264`                | Empty codec name (ffprobe omitted it) → **no token**. |
| `audio_codec`      | `audio_codec:aac`                | No audio stream → `audio_codec:none` (a real signal). |
| `aspect_ratio`     | `aspect:16:9`, `aspect:4:3`      | Derived: `w:h` reduced by GCD. The colon in the value is fine — the whole string is the token id. |
| `fps`              | `fps:24`, `fps:23.976`, `fps:25`, `fps:29.97`, `fps:30`, `fps:50`, `fps:60` | Snap to nearest standard rate within ±2% tolerance; otherwise fall back to a coarse bucket `fps:low` (<30) / `fps:mid` (30–50) / `fps:high` (≥50). `fps` is `None` → no token. |

Categorical features are **not** smoothed or bucketed; they are already
low-cardinality and directly comparable across files.

Continuous, near-unique features emit **neighbor-singleton tokens** (next two sections):

| Feature     | Derived how                    | Token prefix  |
|-------------|--------------------------------|---------------|
| `duration`  | `duration_secs` (raw)          | `duration:`   |
| `filesize`  | `file_size` (raw)              | `filesize:`   |
| `bitrate`   | `file_size * 8 / duration_secs`| `bitrate:`    |

## Bucketing

`duration`, `filesize`, and `bitrate` are near-unique per title, so a raw-value
token would be a singleton vocabulary entry that never generalizes. They are
bucketed **geometrically** with base `B` (default **1.5**):

```
bucket(v) = floor( log_B( max(v, 1.0) ) )      // v in seconds / bytes / bits-per-second
```

- **Why base 1.5, not 2.** Power-of-2 buckets are too coarse for media: a
  factor-of-2 duration bucket lumps a 90-minute film with a 3-hour epic, and a
  factor-of-2 size bucket lumps a 4 GB rip with an 8 GB remux. Base 1.5 yields
  ~22 duration buckets across a realistic 1 s–4 h range and ~56 size buckets
  across 1 B–64 GB — enough resolution to separate meaningful media tiers
  (TV episode vs. short film vs. feature vs. epic) without exploding the
  vocabulary. Empty low buckets simply never mint tokens, so a fine base costs
  nothing for ranges the library doesn't cover.
- The bucket index is a **plain integer label** in the token string
  (`duration:21`, not `duration:5m-10m`). Human-readable ranges are a *display*
  concern; the token is the index. (`ffprobe-cache.md` keeps the raw value, so a
  future TUI can always map index `i` back to `[B^i, B^(i+1))` for display.)
- `v <= 0` (no data) → **no token** for that feature.

The base is configurable so a library with a narrow value distribution can
tighten resolution or a huge one can widen it:

```
--features-bucket-base <B>     # default 1.5, must be > 1.0
```

## Smoothing — neighbor singleton tokens

This is the core of the design and the part that makes nearby buckets "boost or
lower" each other.

### Scheme

For a continuous value in bucket `i`, with **smoothing half-width** `w`
(default **1**), emit the singleton bucket token `feature:k` for **every bucket
`k` in `[i-w, i+w]`** (clamped to `≥ 0`, deduplicated). At the default `w = 1`
this is exactly three tokens per continuous feature:

```
duration:(i-1)      duration:i      duration:(i+1)
```

i.e. the bucket itself plus its immediate left and right neighbors — the
`bucket:2 + bucket:3 + bucket:4` scheme for a file in bucket 3.

### Why this beats edge/interval tokens

An earlier draft proposed *interval* tokens — `feature:i` plus two *edge*
tokens `feature:(i-1)-i` and `feature:i-(i+1)` (one per bucket boundary), meant
to "treat each edge separately." On closer analysis that separateness is
**structurally redundant**: a file in bucket `i` *always* emits both edges
together (the pair is a deterministic function of `i`), so the model can never
observe one without the other. The edges distinguish *which bucket you are in
relative to `i`* — which the singleton `feature:i` already encodes. Splitting
the coupling into two directional tokens adds no information; it only inflates
the vocabulary. The neighbor-singleton scheme gets the same coupling from
fewer, simpler tokens.

Concretely, compare the two at `w = 1` (both emit **3 tokens per file**):

| buckets apart (`d`) | shared tokens — interval scheme | shared tokens — neighbor scheme |
|---|---|---|
| 0 (same bucket)   | 3 | 3 |
| 1 (adjacent)      | 1 (`edge(i,i+1)`) | 2 (`s_i`, `s_{i+1}`) |
| 2                 | 0 | 1 (`s_{i+1}`) |
| ≥ 3               | 0 | 0 |

The neighbor scheme is a **wider, smoother triangular kernel** —
`max(0, 2w+1 - d)` shared tokens at distance `d` — for the same per-file token
cost, and with a **smaller distinct vocabulary** (`n` singletons vs `~2n-1`
singletons-plus-edges).

### Per-token estimates are boxcar-smoothed

The decisive property for *sparse* media data: under the neighbor scheme the
trained count of singleton `s_k` is the number of training files in buckets
`[k-w, k+w]` — a **`(2w+1)`-bucket boxcar sum** of the raw histogram. So every
feature token's `P(token | class)` is already a smoothed, low-variance estimate
even when individual buckets hold only one or two files. The interval scheme's
singleton, by contrast, is a sharp 1-bucket estimate (high variance under
sparsity); only its edge tokens borrow neighboring mass, and only across a
2-bucket boundary. For near-unique continuous values — the exact case that
motivated bucketing — the boxcar robustness of the neighbor scheme is the more
valuable property.

### Resolution trade-off (the honest loss)

The neighbor scheme has **no sharp singleton**: every token is a smeared
`(2w+1)`-bucket sum, so a sharp peak in the true distribution (many files in
exactly one bucket) is blurred across `2w+1` buckets. The interval scheme
retains a 1-bucket singleton and can represent such a peak sharply. For media
features this is a poor trade: duration/size/bitrate distributions are broad
and smooth, and per-bucket counts are small, so peak resolution is rarely
valuable while variance reduction is. If a feature ever does need sharp
resolution, set `--features-smoothing=0` (plain 1-bucket singletons, no
neighbor coupling) for that case.

A second, milder caveat: at `w = 1` the kernel reaches `d = 2` (a factor of
`B^2 = 2.25×` between coupled buckets for base 1.5). If only *immediate*
(`d = 1`) coupling is desired, the interval scheme's `[3,1,0]` is tighter than
the neighbor scheme's `[3,2,1,0]`. In practice the `d = 2` coupling has weight
only 1 (vs 3 at `d = 0`), so the over-smoothing is mild; tighten it by lowering
`--features-bucket-base` if it ever matters.

### General half-width `w`

Emit singletons `s_{i-w}, …, s_i, …, s_{i+w}` — `2w+1` tokens per continuous
feature (fewer at the lower edge, where indices clamp to 0 and dedup). The
shared-token kernel is `max(0, 2w+1 - d)`: triangular, peak `2w+1` at `d = 0`,
decaying linearly to 0 at `d = 2w+1`. `w = 0` disables smoothing (plain
1-bucket singletons, no neighbor coupling).

```
--features-smoothing <w>      # default 1; 0 = singleton buckets only, no neighbor coupling
```

### Edge cases

- The `max(v, 1.0)` clamp in `bucket()` forces `i ≥ 0`, so neighbor indices
  `i-w` that would go negative are clamped to 0 and deduplicated — a file in
  bucket 0 emits `{s_0, s_1}` (two tokens), not a phantom `s_{-1}`. No
  special-casing beyond clamp-and-dedup.
- A bucket index far outside the library's observed range just never coincides
  with a trained token; Naive Bayes' Laplace smoothing gives it the neutral
  `(1)/(vocab + totals)` likelihood, so out-of-range values degrade gracefully.

## Combinations across features

After expanding each feature into its tokens (categorical singletons +
per-continuous-feature neighbor singletons), the whole feature token vector is fed
to `Ngrams::combinations` with order `k = --features-combinations` (default
**2**), producing orderless cross-feature ngrams:

```
{video_codec:h264, duration:21}
{aspect:16:9, bitrate:39}
{audio_codec:ac3, filesize:56, duration:21}     // k = 3
```

These capture feature *co-occurrence* (e.g. "h264 + high bitrate + 1080p"
tending to be positive) the same way path-token combinations capture path
co-occurrence. They are orderless, deduplicated, and (during the candidate
pass) filtered against the frequent set exactly as path ngrams are.

```
--features-combinations <n>   # default 2; 0 disables cross-feature combinations
```

### Same-feature self-combinations

`Ngrams::combinations` combines *all* non-special tokens, so a file in bucket
`i` also produces self-combinations like `{duration:i, duration:i+1}`.
These are redundant (the neighbor singletons are deterministic functions of the
bucket index) but **harmless**: they appear identically for every file in bucket
`i`, so they're consistently double-counted across train and score and Laplace
smoothing absorbs the redundancy. The default keeps the simple reuse of the
existing `combinations` code. A future `--features-cross-only` flag could tag
each token with its feature family and restrict combinations to *different*
families if the redundant-vocabulary cost ever matters.

### Cost

A typical file emits ~1 (`video_codec`) + ~1 (`audio_codec`) + ~1 (`aspect`) +
~1 (`fps`) + 3 (`duration`) + 3 (`filesize`) + 3 (`bitrate`) ≈ **13 feature
tokens**. At `--features-combinations=2` that's `C(13,2) + 13 ≈ 91` ngrams
before dedup/filter; at `k=3`, `≈ 300`. Because continuous features emit only singletons (not singletons-plus-edges),
the distinct feature-token vocabulary is roughly `n` buckets per continuous
feature rather than `~2n`. Filtered against the frequent set (as path ngrams
already are), the surviving vocabulary stays modest. `k=2` is the recommended
default.

## The classifier: the existing `NaiveBayesClassifier`, merged

Feature ngrams are **appended into the same per-entry `Ngrams` vec** as the
path ngrams and consumed by the **single existing `NaiveBayesClassifier`** —
no new struct, no second instance, no extra score column.

This is sound because the classifier is **Bernoulli**: `train_positive` /
`train_negative` iterate the `Ngrams` vec (which `windows` + `combinations`
have already sort-and-dedup'd) and add **1 per unique ngram**, and
`calculate_score` sums log-likelihoods over **present** ngrams. Document-local
frequency never enters the model. So a merged vec `{path ngrams} ∪ {feature
ngrams}` is just a larger set of binary features for the same document; the
dedup in `Ngrams` guarantees each ngram — whether it originated from a path
window, a path combination, or a feature combination — contributes exactly
once. There is no double-counting even when the same `Ngram` hash is produced
by more than one of the three generation passes (they collapse to one entry).

- **Training** (`App::train_naive_bayes_classifier`): for each playlist entry,
  build its merged `Ngrams` (path windows + path combinations + feature
  combinations — see *Pipeline*) and call
  `naive_bayes.train_positive|train_negative(&ngrams)` exactly as today; the
  classifier is unaware that some ngrams came from features.
- **Scoring** (`get_classifiers`): unchanged. The single `naive_bayes` column
  reads `entry.ngrams` (now containing feature ngrams too) and is normalized
  alongside file_size / dir_size / file_age in
  `calculate_scores_and_sort_entries`. No new column, no `get_classifiers`
  change.
- **Neutral when no features.** Files whose probe failed have
  `features == MediaFeatures::default()` (all zeros / empty). Such files
  produce an **empty feature token vector**, so the third `combinations` call
  appends nothing and `entry.ngrams` is just the path ngrams — the classifier
  scores them exactly as it does today, so missing features never distort
  ranking. (Concretely: a probe failure means `duration_secs == 0.0`, so
  `bucket(0) → no token`, and empty/zero categorical fields emit nothing by
  the rules above; only `audio_codec:none` would fire if we treated "no audio"
  as present — but a probe failure has a video stream by definition, so
  `audio_codec` is whatever ffprobe reported, possibly empty → no token. Net:
  no feature ngrams appended.)

### Why merge into the path NB

The earlier draft kept a **separate** `NaiveBayesClassifier` instance over a
private feature ngram vec, citing three concerns. On re-examination against the
actual `Ngrams` / `NaiveBayesClassifier` mechanics, none of them justify the
second instance:

1. **"Couples two independent feature spaces."** It does — and that is the
   point. The total score is meant to combine path and media signal; a single
   Bernoulli NB over the union of ngrams is exactly a linear-in-log-space
   combination of the two feature sets, with one shared prior. The coupling is
   the desired behavior, not a hazard.
2. **"Makes the path NB's prior/normalization depend on feature presence."**
   The prior `log P(class) = log((1+pos)/(2+pos+neg))` depends only on class
   counts, not on which ngrams are present, so it is unaffected by merging.
   The Laplace denominator `(total_ngrams + vocab_size)` does grow to include
   feature ngrams, which uniformly rescales every per-ngram log-likelihood
   (path and feature alike). This dilutes magnitudes slightly but preserves
   the relative ordering of ngrams within each class and across classes; it is
   still a valid NB. The cost is mild and is the price of one model instead of
   two.
3. **"Prevents toggling features independently."** Toggling is already
   available via the feature CLI flags (`--features-combinations=0
   --features-smoothing=0` collapses features to categorical singletons; an
   empty probe result appends nothing). A future `--no-features` can skip the
   feature `combinations` call entirely without a second classifier.

The decisive mechanic is the one this section leads with: Bernoulli training
over a deduplicated vec means a merged vec is *just more binary features*,
handled identically to path ngrams. Keeping a second instance would duplicate
the model, the training loop, and a score column for no statistical benefit.

### Why no windows for features

`Ngrams::windows` generates *contiguous* subsequences of the token slice.
Features have no meaningful order — `{video_codec:h264, duration:21}` is the
same signal regardless of which is "first" — so contiguous windows would be
arbitrary and order-dependent. Only `Ngrams::combinations` (which sorts each
combo before hashing) is order-invariant and therefore correct for features.
The feature pipeline calls `combinations` only; `windows` is never invoked on
feature tokens. This is why `--features-combinations` is independent of
`--combinations` and there is no `--features-windows`.

## Pipeline

The feature pipeline runs in memory, after the ffprobe cache populate (which
guarantees every entry's `features` is populated) and alongside the existing
tokenize → ngram → train phases:

```rust
/// Build the feature token vector for one file's MediaFeatures.
fn feature_tokens(f: &MediaFeatures, map: &mut TokenMap,
                  bucket_base: f64, smoothing: usize) -> Vec<Token> {
    let mut out = Vec::new();

    // --- Categorical / derived ---
    if !f.video_codec.is_empty() {
        out.push(map.get_or_create_token(&format!("video_codec:{}", f.video_codec)));
    }
    if f.audio_codec.is_empty() {
        out.push(map.get_or_create_token("audio_codec:none"));
    } else {
        out.push(map.get_or_create_token(&format!("audio_codec:{}", f.audio_codec)));
    }
    let (aw, ah) = reduce_ratio(f.width, f.height); // GCD reduction
    out.push(map.get_or_create_token(&format!("aspect:{}:{}", aw, ah)));
    if let Some(fps) = f.fps {
        if let Some(label) = snap_fps(fps) {        // "24", "23.976", ..., or coarse
            out.push(map.get_or_create_token(&format!("fps:{}", label)));
        }
    }

    // --- Continuous: bucket + neighbor singletons ---
    if f.duration_secs > 0.0 {
        emit_neighbors(&mut out, map, "duration", f.duration_secs,
                       bucket_base, smoothing);
    }
    if f.file_size > 0 {
        emit_neighbors(&mut out, map, "filesize", f.file_size as f64,
                       bucket_base, smoothing);
    }
    if f.duration_secs > 0.0 {
        let bitrate = (f.file_size as f64) * 8.0 / f.duration_secs;
        if bitrate.is_finite() && bitrate > 0.0 {
            emit_neighbors(&mut out, map, "bitrate", bitrate,
                           bucket_base, smoothing);
        }
    }
    out
}

/// Push neighbor-singleton tokens for one continuous feature.
/// Emits feature:k for every k in [i-w, i+w] (clamped to >=0, deduped),
/// where i = bucket(v). w=0 yields just feature:i (plain bucketing).
fn emit_neighbors(out: &mut Vec<Token>, map: &mut TokenMap,
                  name: &str, v: f64, base: f64, w: usize) {
    if v < 1.0 { return; }                      // clamp: no sub-1 buckets
    let i = (v.ln() / base.ln()).floor() as i64;
    if i < 0 { return; }
    let lo = (i - w as i64).max(0);
    let hi = i + w as i64;
    for k in lo..=hi {
        out.push(map.get_or_create_token(&format!("{}:{}", name, k)));
    }
}
```

`feature_tokens` is called once per entry to build a `Tokens` (root + feature
tokens + eol, bloom unused) and then a **third** `Ngrams::combinations` call
appends the feature ngrams into the **same** `Ngrams` vec that already holds
the path windows + path combinations:

```rust
let map = tokenizer.token_map_mut(); // &mut TokenMap shared with paths
let mut ft = Tokens::default();
ft.tokens.push(map.root());
ft.tokens.extend(feature_tokens(&entry.features, map, base, w));
ft.tokens.push(map.eol());

// entry.ngrams already holds path windows + path combinations; this appends
// feature combinations and re-runs the sort-and-dedup, so the merged vec is
// free of duplicates across all three generation passes.
entry.ngrams.as_mut().unwrap().combinations(
    &ft, features_combinations, map.last_special(),
    frequent_ngrams.as_ref(), None,
);
```

The feature `combinations` call reuses the **same** `frequent_ngrams` allowed
set as the path passes. Because feature tokens share the path `TokenMap`,
feature ngram hashes live in the same id space and are admitted by the same
filter; rare cross-feature combos are pruned exactly as rare path ngrams are.
This requires the frequent-set counting pass to count feature ngrams too — see
*Frequent-set counting* below. The full phase order in `App`:

1. ffprobe cache populate → every `entry.features` populated (already exists).
2. Build `PairTokenizer` from paths → `entry.tokens` (already exists).
3. **NEW:** For each entry compute `feature_tokens` against the tokenizer's
   shared `TokenMap` and **append** its combinations into `entry.ngrams`
   (the same vec that holds path windows + path combinations).
4. Count frequent ngrams over the merged space (path + feature); store
   `entry.ngrams` (paths + features, windows+combinations, deduped together).
5. Train the single `naive_bayes` on `entry.ngrams` (now containing feature
   ngrams too) — unchanged training loop.
6. `calculate_scores_and_sort_entries` normalizes the existing columns and
   sums; no new column.

### Frequent-set counting

`Ngrams::count_and_filter_from_paths` currently tokenizes paths only. To keep
the merged `frequent_ngrams` filter valid for feature ngrams, extend the
counting pass to also generate feature combinations per entry (using the same
shared `TokenMap` and the same `features_combinations` order) and fold those
counts into the same `AHashMap<Ngram, u8>`. The filter threshold (count > 1)
and the final `AHashSet<Ngram>` are unchanged; the set simply now contains
frequent feature ngrams alongside frequent path ngrams. Alternatively, pass
`None` for `allowed` in the feature `combinations` call to disable frequency
filtering for features only — at the cost of retaining rare cross-feature
combos. The merged approach is recommended.

## CLI flags

Added to `CommonArgs` (visible to `build` and `score`):

```
--features-combinations <n>   # default 2; orderless cross-feature combo order; 0 disables
--features-smoothing <w>      # default 1; continuous-bucket neighbor half-width; 0 = singleton only
--features-bucket-base <B>    # default 1.5; geometric bucket base for duration/filesize/bitrate; > 1.0
```

There is deliberately **no `--features-bias`**: features feed the same
`naive_bayes` column as paths (which also has no bias flag — it's always on),
so there is no separate column to bias. Disabling features entirely is done by
`--features-combinations=0 --features-smoothing=0` (yields only categorical
singletons) or, for a fully neutral contribution, by leaving all entries'
features unset (probe failures / empty cache) — both simply cause the feature
`combinations` call to append little or nothing. A future `--no-features` flag
would just skip that call. A `--features-weight` is noted under *Future work*;
it would require splitting features back out into their own column, so it is
not the default design.

## `Entry` / module changes

```rust
// app.rs
pub struct Entry {
    // ... existing fields ...
    pub ngrams: Option<Ngrams>, // now also holds appended feature ngrams
    pub scores: Box<[f64]>,     // unchanged size — no extra column
    pub features: MediaFeatures,
}
```

No new field, no new score column. `entry.ngrams` simply accumulates feature
combinations alongside the path windows/combinations it already holds;
`get_classifiers` and the `scores` sizing are unchanged.

### Module layout

```
src/
  features.rs     // NEW: feature_tokens, emit_neighbors, reduce_ratio,
                  //   snap_fps, bucket(), and the merged count-frequent
                  //   helper. Pure functions over MediaFeatures + the shared
                  //   TokenMap; no I/O, no classifier state.
  classifier.rs   // unchanged — no second instance; the single
                  //   NaiveBayesClassifier consumes merged ngrams.
  ngrams.rs        // unchanged — features reuse Ngrams::combinations, which
                  //   already appends + dedups into the same vec.
  tokens.rs        // unchanged — features reuse the tokenizer's TokenMap.
  app.rs           // feature pipeline phase appends into entry.ngrams;
                  //   no new field, no get_classifiers change.
  main.rs          // three new CommonArgs flags.
```

`features.rs` depends only on `crate::cache::MediaFeatures` and
`crate::tokens::TokenMap` (and `crate::ngrams::Ngrams` for the count-frequent
helper). It holds no state and is fully unit-testable with stub
`MediaFeatures` (the existing `ffprobe.rs` tests already build `MediaFeatures`
by hand, so test fixtures are trivial).

## Settled decisions

- **Base 1.5, not 2.** Power-of-2 buckets are too coarse for media; 1.5 keeps
  meaningful tiers distinct without vocabulary blowup. Configurable via
  `--features-bucket-base`.
- **Neighbor-singleton smoothing (`w=1` default).** Emits `feature:(i-1)` +
  `feature:i` + `feature:(i+1)` — the bucket plus its immediate neighbors. Chosen
  over an *interval/edge* scheme (`feature:i` + two directional edge tokens)
  because directional edges are structurally redundant (a file in bucket `i`
  always emits both edges, so the pair encodes nothing beyond the singleton) and
  inflate the vocabulary. The neighbor scheme gives a wider triangular kernel
  (`max(0, 2w+1-d)` shared tokens at distance `d`) for the same per-file token
  count, a smaller distinct vocabulary, and boxcar-smoothed per-token estimates
  that are more robust under sparsity — the decisive property for near-unique
  continuous values. See *Smoothing*. Generalizes to `w ≥ 2` and `w = 0`
  (plain bucketing).
- **Bucket index as the token label.** Human-readable ranges are a display
  concern; the raw value is retained in the cache so display can always recover
  `[B^i, B^(i+1))`. Avoids tying token identity to a formatting choice.
- **Merge into the existing `NaiveBayesClassifier`, not a separate instance.**
  Feature ngrams are appended into the same per-entry `Ngrams` vec (deduped
  with path windows/combinations) and consumed by the single existing
  classifier. Sound because the model is Bernoulli (presence/absence, deduped
  vec → 1 count per unique ngram) and because feature tokens share the path
  `TokenMap` (so ngram hashes never falsely collide across the two spaces).
  Avoids a second instance, a second training loop, and an extra score column;
  the only cost is a mild, uniform Laplace-denominator dilution. See *Why
  merge into the path NB*.
- **Combinations only, never windows, for features.** Features are orderless;
  contiguous windows would be arbitrary. `--features-combinations` is
  independent of the path `--combinations`; there is no `--features-windows`.
- **Same-feature self-combinations allowed by default.** Redundant but
  harmless; keeps the existing `Ngrams::combinations` reuse. A future
  `--features-cross-only` flag can prune them if needed.
- **Raw-on-disk preserved.** Everything here is in-memory at read time; no
  persisted buckets, intervals, or derived values, consistent with
  `ffprobe-cache.md`'s canonical raw-on-disk principle.

## Future work

- **`--features-cross-only`**: tag each feature token with its family and
  restrict `Ngrams::combinations` to cross-family combos, dropping redundant
  same-feature self-combinations.
- **`--features-weight <w>` / `--no-features`**: explicit weighting or disabling
  of the feature contribution without zeroing the flags. `--no-features` simply
  skips the feature `combinations` call; `--features-weight` would require
  splitting features back into their own score column (reversing the merge), so
  it is a larger change and not the default.
- **Per-feature bucket bases**: duration, filesize, and bitrate have very
  different scales and distributions; a per-feature base (or an offset before
  the log, mirroring `FileSizeClassifier`'s `file_size_offset`) could tighten
  resolution where it matters.
- **Adaptive / quantile buckets**: geometric buckets assume a sensible scale
  invariant; a data-driven quantile scheme could auto-fit boundaries to the
  observed library distribution (still in-memory, still raw-on-disk).
- **Kernel weighting**: the current scheme is a hard triangular kernel
  (shared-token counts). A soft variant could weight each neighbor token by its
  kernel value, but that requires leaving pure Bernoulli NB; deferred.
- **`fps` as a smoothed continuous feature**: if standard-rate snapping proves
  too lossy, `fps` could instead be bucketed + neighbor-smoothed like duration.
  Currently categorical because real-world fps clusters tightly at standard
  rates, making categorical snapping the higher-signal choice.
