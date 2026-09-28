//! A reserved arena for the vectors an agent loop keeps asking about again.
//!
//! Plain words: an agent asks about a changing state but a mostly fixed set of
//! actions, and it often revisits states it has already seen. Neither their
//! embeddings nor their projections change while the head does not, so they
//! are worth keeping next to the head instead of recomputing on every request.
//! This is `/tmp/CLM/src/clm/cache.py`'s `VectorArena`, minus torch: one flat
//! `Vec<f32>` claimed once at start-up, carved into per-width pools, never
//! grown — a long-running process cannot drift into an out-of-memory kill.
//!
//! # Design (mirrors `cache.py`)
//!
//! - The arena is claimed once: a budget is either a fraction of the device
//!   (`"0.02"`, vLLM-style) or an absolute size (`"512MiB"`); `"0"` disables
//!   the cache. Bare numbers must be in `[0, 1)` (`parse_budget`).
//! - Pools of different widths are carved out of the one allocation at
//!   start-up (512-d projections for heads, 4096-d encoder embeddings for a
//!   raw-space ablation); `reserve` after start-up for an already-carved
//!   width is a no-op, and a width that does not fit its share is bypassed.
//! - Entries are keyed by `{namespace}\x00{text}`. The namespace names the
//!   head and its generation, so a hot-reloaded head simply stops matching
//!   rows its previous weights produced; least-recently-used eviction
//!   reclaims them.
//! - `get` resolves misses through a caller-supplied `compute` closure run
//!   *outside* the lock (the slow path), then re-resolves under the lock: a
//!   concurrent writer may have evicted a row between fill and resolve, in
//!   which case the request is answered without the arena ("thrashing").
//!
//! # Divergences from `cache.py`
//!
//! - Host memory instead of a torch device tensor: there is no GPU tensor
//!   runtime in this crate. Fractions are taken against [`CPU_DEVICE_BYTES`]
//!   (8 GiB, exactly `cache.py`'s non-CUDA fallback) and additionally capped
//!   at [`ARENA_BYTES_MAX`] (2 GiB) because host memory, unlike a CUDA
//!   device, is shared with everything else on the machine.
//! - `get` returns owned rows (`Vec<Vec<f32>>`), copying out of the arena,
//!   rather than a device tensor view. The copy is bounded by
//!   [`TEXTS_MAX`] * [`DIM_MAX`] and documented here.
//! - The lock is a `std::sync::Mutex` (this crate is synchronous; tokio
//!   integration is future work per the crate docs).
//!
//! # Bounds
//!
//! - [`TEXTS_MAX`] texts per `get`, each at most [`TEXT_BYTES_MAX`] bytes;
//!   namespaces at most [`NAMESPACE_BYTES_MAX`] bytes.
//! - Pool widths are in `1..=DIM_MAX`.
//!
//! # Unsafe policy
//!
//! This module forbids unsafe code (crate-level `#![forbid(unsafe_code)]`).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;

/// Device size assumed for fraction budgets on CPU: 8 GiB, exactly the
/// non-CUDA fallback in `/tmp/CLM/src/clm/cache.py:86-88`.
pub const CPU_DEVICE_BYTES: u64 = 8 << 30;

/// Host-side ceiling for any arena: unlike a CUDA device, host memory is
/// shared with the rest of the machine, so a fraction budget can never claim
/// more than this.
pub const ARENA_BYTES_MAX: u64 = 2 << 30;

/// Default budget when none is given: 2% of the device, vLLM-style
/// (`/tmp/CLM/src/clm/cache.py:29`, `DEFAULT_BUDGET`).
pub const DEFAULT_BUDGET_SPEC: &str = "0.02";

/// Maximum texts resolved in one [`VectorArena::get`] call.
pub const TEXTS_MAX: usize = 4096;

/// Maximum bytes of a single cached text.
pub const TEXT_BYTES_MAX: usize = 1 << 20;

/// Maximum bytes of a cache namespace.
pub const NAMESPACE_BYTES_MAX: usize = 256;

/// Maximum vector width of a pool.
pub const DIM_MAX: usize = 1 << 16;

/// Initial hash-map capacity cap: a huge arena must not pre-size its maps
/// from the row count alone.
const MAP_CAPACITY_MAX: usize = 1 << 20;

/// Truncation bound for error context carried inside [`CacheError`].
const ERROR_CONTEXT_CHARS_MAX: usize = 256;

/// Computes projected vectors for cache misses, in order: text in, one row
/// per text out. Factored out so call sites do not repeat the full type.
/// The explicit lifetime keeps non-`'static` closures (borrowing the
/// embedder, head, and token counter) usable as miss callbacks.
pub type VectorCompute<'a> = dyn FnMut(&[String]) -> Result<Vec<Vec<f32>>, CacheError> + 'a;

/// A parsed arena budget: a fraction of the device, an absolute byte count,
/// or disabled. `Fraction` values outside `[0, 1)` are rejected by
/// [`VectorArena::new`]; construct through [`parse_budget`] when the input
/// is untrusted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Budget {
    /// Do not build an arena.
    Disabled,
    /// Fraction of [`CPU_DEVICE_BYTES`], in `[0, 1)`.
    Fraction(f64),
    /// Absolute byte count.
    Bytes(u64),
}

/// Failures of [`parse_budget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetError {
    /// The numeric part did not parse.
    NotANumber,
    /// The unit suffix is unknown.
    UnknownUnit {
        /// The offending suffix, upper-cased and bounded.
        unit: String,
    },
    /// A bare number was not in `[0, 1)`.
    FractionOutOfRange,
    /// A negative size.
    Negative,
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BudgetError::NotANumber => write!(
                f,
                "cache budget is not a fraction (0.02) or a size (512MiB)"
            ),
            BudgetError::UnknownUnit { unit } => {
                write!(
                    f,
                    "unknown size unit {unit:?}; use B, KB, MB, GB, KiB, MiB or GiB"
                )
            }
            BudgetError::FractionOutOfRange => write!(
                f,
                "a bare number is a fraction of device memory and must be in [0, 1); \
                 give a unit (512MiB) for an absolute size"
            ),
            BudgetError::Negative => write!(f, "cache budget must not be negative"),
        }
    }
}

impl std::error::Error for BudgetError {}

/// Parse `"0.02"` -> 2% of the device, `"512MiB"` / `"2GB"` -> that many
/// bytes, `"0"` -> disabled. `None`/empty -> [`DEFAULT_BUDGET_SPEC`].
/// Mirrors `/tmp/CLM/src/clm/cache.py:36-54`, `parse_budget`.
pub fn parse_budget(spec: Option<&str>) -> Result<Budget, BudgetError> {
    let raw = spec.unwrap_or("").trim();
    let raw = if raw.is_empty() {
        DEFAULT_BUDGET_SPEC
    } else {
        raw
    };
    let cut = raw
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(raw.len());
    let (number, unit) = (raw[..cut].trim(), raw[cut..].trim().to_ascii_uppercase());
    let value: f64 = number.parse().map_err(|_| BudgetError::NotANumber)?;
    if !value.is_finite() || value < 0.0 {
        return Err(if value.is_finite() {
            BudgetError::Negative
        } else {
            BudgetError::NotANumber
        });
    }
    // Same unit table as cache.py: decimal KB/MB/GB, binary KiB/MiB/GiB.
    let multiplier: Option<u64> = match unit.as_str() {
        "" => None,
        "B" => Some(1),
        "KB" => Some(1_000),
        "MB" => Some(1_000_000),
        "GB" => Some(1_000_000_000),
        "KIB" => Some(1 << 10),
        "MIB" => Some(1 << 20),
        "GIB" => Some(1 << 30),
        _ => {
            let bounded: String = unit.chars().take(16).collect();
            return Err(BudgetError::UnknownUnit { unit: bounded });
        }
    };
    match multiplier {
        None => {
            if value == 0.0 {
                Ok(Budget::Disabled)
            } else if value < 1.0 {
                Ok(Budget::Fraction(value))
            } else {
                Err(BudgetError::FractionOutOfRange)
            }
        }
        Some(mult) => {
            let bytes = (value * mult as f64) as u64;
            if bytes == 0 {
                Ok(Budget::Disabled)
            } else {
                Ok(Budget::Bytes(bytes))
            }
        }
    }
}

/// Failures of arena operations. Compute-callback failures surface as
/// [`CacheError::ComputeFailed`] with bounded context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    /// A mutex was poisoned; the guarded state is not trusted.
    LockPoisoned,
    /// The budget disables the cache (mirrors `cache.py`'s `CacheDisabled`,
    /// raised when the budget is `"0"`).
    Disabled,
    /// Pool width invalid.
    BadDim {
        /// The offending width.
        dim: usize,
    },
    /// Reserve share was not in `(0, 1]`.
    BadShare,
    /// Namespace was empty or over [`NAMESPACE_BYTES_MAX`] bytes.
    BadNamespace,
    /// Too many texts in one `get`.
    TooManyTexts {
        /// Texts offered.
        count: usize,
    },
    /// A text exceeded [`TEXT_BYTES_MAX`] bytes.
    TextTooLong {
        /// Byte length of the offending text.
        bytes: usize,
    },
    /// The compute callback returned the wrong number of rows.
    ComputeArity {
        /// Rows expected (miss count).
        expected: usize,
        /// Rows returned.
        got: usize,
    },
    /// A computed row violated the vector contract (wrong width or
    /// non-finite component); nothing was inserted.
    BadVector {
        /// Miss index of the offending row.
        index: usize,
    },
    /// The compute callback failed; bounded context.
    ComputeFailed {
        /// Bounded failure context.
        context: String,
    },
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::LockPoisoned => write!(f, "vector arena lock poisoned"),
            CacheError::Disabled => write!(f, "vector arena disabled by budget 0"),
            CacheError::BadDim { dim } => write!(f, "invalid pool width {dim}"),
            CacheError::BadShare => write!(f, "reserve share must be in (0, 1]"),
            CacheError::BadNamespace => {
                write!(f, "namespace must be 1..=NAMESPACE_BYTES_MAX bytes")
            }
            CacheError::TooManyTexts { count } => {
                write!(f, "{count} texts exceeds TEXTS_MAX={TEXTS_MAX}")
            }
            CacheError::TextTooLong { bytes } => {
                write!(
                    f,
                    "text of {bytes} bytes exceeds TEXT_BYTES_MAX={TEXT_BYTES_MAX}"
                )
            }
            CacheError::ComputeArity { expected, got } => {
                write!(f, "compute returned {got} rows for {expected} misses")
            }
            CacheError::BadVector { index } => {
                write!(
                    f,
                    "compute row {index} had wrong width or non-finite components"
                )
            }
            CacheError::ComputeFailed { context } => {
                write!(f, "vector compute failed: {context}")
            }
        }
    }
}

impl std::error::Error for CacheError {}

/// Bound a message for inclusion in [`CacheError::ComputeFailed`].
fn bound_context(message: &str) -> String {
    message.chars().take(ERROR_CONTEXT_CHARS_MAX).collect()
}

/// One width of vector inside the arena: a fixed row count, LRU.
/// Mirrors `/tmp/CLM/src/clm/cache.py:57-84`, `Pool`.
struct Pool {
    dim: usize,
    /// Offset of this pool's first row inside the arena's flat buffer.
    base: usize,
    rows: usize,
    /// Cache key -> slot index.
    slots: HashMap<String, usize>,
    /// Keys from least to most recently used.
    lru: VecDeque<String>,
    /// Never-issued slot indices.
    free: Vec<usize>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl Pool {
    fn new(dim: usize, base: usize, rows: usize) -> Self {
        Self {
            dim,
            base,
            rows,
            slots: HashMap::with_capacity(rows.min(MAP_CAPACITY_MAX)),
            lru: VecDeque::with_capacity(rows.min(MAP_CAPACITY_MAX)),
            free: (0..rows).rev().collect(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Mark `key` most-recently-used. `key` must be present.
    fn touch(&mut self, key: &str) {
        if let Some(position) = self.lru.iter().position(|k| k == key) {
            self.lru.remove(position);
        }
        self.lru.push_back(key.to_string());
    }

    /// Claim a slot for `key`, evicting the least-recently-used entry when
    /// full. Mirrors `Pool.claim` (`cache.py:66-73`).
    fn claim(&mut self, key: String) -> usize {
        let slot = match self.free.pop() {
            Some(free) => free,
            None => {
                // Invariant: no free slots means every row is claimed, so the
                // LRU list is non-empty and agrees with the slot map.
                let oldest = self.lru.pop_front().expect("full pool tracks an LRU key");
                let reused = self.slots.remove(&oldest).expect("LRU key has a slot");
                self.evictions += 1;
                reused
            }
        };
        self.slots.insert(key.clone(), slot);
        self.lru.push_back(key);
        slot
    }

    /// Claim a slot for `key`, or touch the existing entry when a concurrent
    /// writer inserted it between probe and commit. Returns `None` when the
    /// key was already present: blindly calling [`claim`][Self::claim] here
    /// would orphan the previously claimed slot (the slots map entry is
    /// overwritten, so the old slot index is never freed or evicted).
    fn claim_or_touch(&mut self, key: String) -> Option<usize> {
        if self.slots.contains_key(&key) {
            self.touch(&key);
            None
        } else {
            Some(self.claim(key))
        }
    }
}

/// Per-pool counters for [`ArenaStats`].
#[derive(Debug, Clone, PartialEq)]
pub struct PoolStats {
    /// Vector width.
    pub dim: usize,
    /// Row capacity.
    pub capacity: usize,
    /// Rows currently claimed.
    pub used: usize,
    /// Reserved megabytes for this pool.
    pub reserved_mb: f64,
    /// Cache hits.
    pub hits: u64,
    /// Cache misses (rows computed and inserted).
    pub misses: u64,
    /// LRU evictions.
    pub evictions: u64,
    /// `hits / (hits + misses)`, or `None` when nothing was asked.
    pub hit_rate: Option<f64>,
}

/// Arena-wide counters. Mirrors `VectorArena.stats` (`cache.py:158-163`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArenaStats {
    /// Reserved megabytes for the whole flat buffer.
    pub reserved_mb: f64,
    /// Overall hit rate, or `None` when nothing was asked.
    pub hit_rate: Option<f64>,
    /// Per-width pool stats, ascending by width.
    pub pools: Vec<PoolStats>,
}

struct ArenaInner {
    flat: Vec<f32>,
    cursor: usize,
    pools: HashMap<usize, Pool>,
    reserved_bytes: usize,
}

/// A single preallocated vector allocation, carved into per-width LRU pools,
/// never grown. Mirrors `/tmp/CLM/src/clm/cache.py:87-163`, `VectorArena`.
pub struct VectorArena {
    inner: Mutex<ArenaInner>,
}

impl VectorArena {
    /// Claim the arena up front: its cost is paid at start-up or not at all
    /// (`cache.py`, `Engine._reserve` docstring). `Err(CacheError::Disabled)`
    /// when the budget disables the cache or cannot hold a single row;
    /// `Err(CacheError::BadShare)` when a directly constructed
    /// [`Budget::Fraction`] lies outside `[0, 1)`.
    pub fn new(budget: Budget) -> Result<Self, CacheError> {
        let want: u64 = match budget {
            Budget::Disabled => return Err(CacheError::Disabled),
            Budget::Fraction(fraction) => {
                if !fraction.is_finite() || fraction < 0.0 || fraction >= 1.0 {
                    return Err(CacheError::BadShare);
                }
                (fraction * CPU_DEVICE_BYTES as f64) as u64
            }
            Budget::Bytes(bytes) => bytes,
        };
        if want == 0 {
            return Err(CacheError::Disabled);
        }
        // Host-side guard: never take more than ARENA_BYTES_MAX (divergence
        // from cache.py documented in the module docs).
        let reserved_bytes = want.min(ARENA_BYTES_MAX) as usize;
        let cells = reserved_bytes / 4;
        if cells == 0 {
            return Err(CacheError::Disabled);
        }
        Ok(Self {
            inner: Mutex::new(ArenaInner {
                flat: vec![0.0f32; cells],
                cursor: 0,
                pools: HashMap::new(),
                reserved_bytes,
            }),
        })
    }

    /// Carve `share` of the arena into rows of `dim`. Call at start-up, once
    /// per width. Returns `Ok(false)` when the width is already carved or
    /// does not fit its share (bypassed, like `cache.py:98-110`); `Ok(true)`
    /// when a pool was carved.
    pub fn reserve(&self, dim: usize, share: f64) -> Result<bool, CacheError> {
        if dim == 0 || dim > DIM_MAX {
            return Err(CacheError::BadDim { dim });
        }
        if !share.is_finite() || share <= 0.0 || share > 1.0 {
            return Err(CacheError::BadShare);
        }
        let mut inner = self.inner.lock().map_err(|_| CacheError::LockPoisoned)?;
        if inner.pools.contains_key(&dim) {
            return Ok(false);
        }
        let total = inner.flat.len();
        let mut rows = (total as f64 * share) as usize / dim;
        let mut end = inner.cursor + rows * dim;
        if end > total {
            rows = (total - inner.cursor) / dim;
            end = inner.cursor + rows * dim;
        }
        if rows == 0 {
            return Ok(false);
        }
        let pool = Pool::new(dim, inner.cursor, rows);
        inner.cursor = end;
        inner.pools.insert(dim, pool);
        Ok(true)
    }

    /// Rows for `texts` under `namespace`: arena hits where possible,
    /// `compute` fills the misses (in order) outside the lock. Mirrors
    /// `VectorArena.get` (`/tmp/CLM/src/clm/cache.py:113-155`).
    ///
    /// Keys are `{namespace}\x00{text}` (`cache.py:122`).
    pub fn get(
        &self,
        namespace: &str,
        dim: usize,
        texts: &[String],
        compute: &mut VectorCompute<'_>,
    ) -> Result<Vec<Vec<f32>>, CacheError> {
        if namespace.is_empty() || namespace.len() > NAMESPACE_BYTES_MAX {
            return Err(CacheError::BadNamespace);
        }
        if dim == 0 || dim > DIM_MAX {
            return Err(CacheError::BadDim { dim });
        }
        if texts.len() > TEXTS_MAX {
            return Err(CacheError::TooManyTexts { count: texts.len() });
        }
        for text in texts {
            if text.len() > TEXT_BYTES_MAX {
                return Err(CacheError::TextTooLong { bytes: text.len() });
            }
        }
        let keys: Vec<String> = texts
            .iter()
            .map(|text| format!("{namespace}\x00{text}"))
            .collect();

        // Phase 1: probe under the lock; collect misses, deduplicated.
        let (missing_texts, missing_keys) = {
            let mut inner = self.inner.lock().map_err(|_| CacheError::LockPoisoned)?;
            let pool = match inner.pools.get_mut(&dim) {
                // No pool for this width: bypass the arena, like cache.py.
                None => return compute(texts),
                Some(pool) => pool,
            };
            let mut missing_texts = Vec::new();
            let mut missing_keys = Vec::new();
            for (text, key) in texts.iter().zip(keys.iter()) {
                if pool.slots.contains_key(key) {
                    pool.touch(key);
                    pool.hits += 1;
                } else if !missing_keys.iter().any(|k| k == key) {
                    missing_texts.push(text.clone());
                    missing_keys.push(key.clone());
                }
            }
            (missing_texts, missing_keys)
        };

        // Phase 2: compute misses outside the lock (the slow path).
        if !missing_keys.is_empty() {
            let vectors = compute(&missing_texts)?;
            if vectors.len() != missing_keys.len() {
                return Err(CacheError::ComputeArity {
                    expected: missing_keys.len(),
                    got: vectors.len(),
                });
            }
            // Validate before inserting: rejected validation leaves published
            // state unchanged.
            for (index, row) in vectors.iter().enumerate() {
                if row.len() != dim || row.iter().any(|x| !x.is_finite()) {
                    return Err(CacheError::BadVector { index });
                }
            }
            // Phase 3: insert under the lock. Recheck each key: a concurrent
            // writer may have inserted it while the rows were computed
            // outside the lock, in which case reuse its slot (touch) instead
            // of claiming a second one.
            let mut inner = self.inner.lock().map_err(|_| CacheError::LockPoisoned)?;
            for (key, row) in missing_keys.iter().zip(vectors.iter()) {
                let slot = inner
                    .pools
                    .get_mut(&dim)
                    .expect("pool carved at phase 1 is never removed")
                    .claim_or_touch(key.clone());
                if let Some(slot) = slot {
                    let base = inner
                        .pools
                        .get(&dim)
                        .expect("pool carved at phase 1 is never removed")
                        .base;
                    let start = base + slot * dim;
                    inner.flat[start..start + dim].copy_from_slice(row);
                }
            }
            inner
                .pools
                .get_mut(&dim)
                .expect("pool carved at phase 1 is never removed")
                .misses += missing_keys.len() as u64;
        }

        // Phase 4: resolve under the lock. A concurrent writer may have
        // evicted a row since it was filled; then answer without the arena
        // ("thrashing", cache.py:148-150).
        let inner = self.inner.lock().map_err(|_| CacheError::LockPoisoned)?;
        let slots: Vec<usize> = {
            let pool = inner
                .pools
                .get(&dim)
                .expect("pool carved at phase 1 is never removed");
            let mut slots = Vec::with_capacity(keys.len());
            for key in &keys {
                match pool.slots.get(key) {
                    Some(&slot) => slots.push(slot),
                    None => {
                        drop(inner);
                        return compute(texts);
                    }
                }
            }
            slots
        };
        {
            let pool = inner
                .pools
                .get(&dim)
                .expect("pool carved at phase 1 is never removed");
            let base = pool.base;
            Ok(slots
                .iter()
                .map(|slot| {
                    let start = base + slot * dim;
                    inner.flat[start..start + dim].to_vec()
                })
                .collect())
        }
    }

    /// Counters for every pool. Mirrors `VectorArena.stats`.
    pub fn stats(&self) -> Result<ArenaStats, CacheError> {
        let inner = self.inner.lock().map_err(|_| CacheError::LockPoisoned)?;
        let mut pools: Vec<PoolStats> = inner
            .pools
            .values()
            .map(|pool| {
                let asked = pool.hits + pool.misses;
                PoolStats {
                    dim: pool.dim,
                    capacity: pool.rows,
                    used: pool.slots.len(),
                    reserved_mb: pool.rows as f64 * pool.dim as f64 * 4.0 / 1e6,
                    hits: pool.hits,
                    misses: pool.misses,
                    evictions: pool.evictions,
                    hit_rate: if asked > 0 {
                        Some(pool.hits as f64 / asked as f64)
                    } else {
                        None
                    },
                }
            })
            .collect();
        pools.sort_by_key(|pool| pool.dim);
        let asked: u64 = pools.iter().map(|pool| pool.hits + pool.misses).sum();
        let hits: u64 = pools.iter().map(|pool| pool.hits).sum();
        Ok(ArenaStats {
            reserved_mb: inner.reserved_bytes as f64 / 1e6,
            hit_rate: if asked > 0 {
                Some(hits as f64 / asked as f64)
            } else {
                None
            },
            pools,
        })
    }

    /// Wrap a compute failure message for [`CacheError::ComputeFailed`].
    pub fn compute_failed(message: &str) -> CacheError {
        CacheError::ComputeFailed {
            context: bound_context(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A 1 MiB arena with one 4-wide pool: 65_536 rows.
    fn arena_4d() -> VectorArena {
        let arena = VectorArena::new(Budget::Bytes(1 << 20)).expect("1 MiB arena builds");
        assert!(arena.reserve(4, 1.0).expect("reserve 4d"));
        arena
    }

    /// Compute closure that counts calls and returns a deterministic row per
    /// text: the row is derived from the text so tests can recognise hits.
    fn counting_compute(
        calls: Rc<Cell<usize>>,
    ) -> impl FnMut(&[String]) -> Result<Vec<Vec<f32>>, CacheError> {
        move |missing: &[String]| {
            calls.set(calls.get() + 1);
            Ok(missing
                .iter()
                .map(|text| {
                    let seed = text.len() as f32;
                    vec![seed, seed + 1.0, seed + 2.0, seed + 3.0]
                })
                .collect())
        }
    }

    // ---- validation: budgets ----

    #[test]
    fn budget_fraction_parses() {
        assert_eq!(parse_budget(Some("0.02")), Ok(Budget::Fraction(0.02)));
    }

    #[test]
    fn budget_absolute_parses() {
        assert_eq!(
            parse_budget(Some("512MiB")),
            Ok(Budget::Bytes(512 * (1 << 20)))
        );
        assert_eq!(parse_budget(Some("2GB")), Ok(Budget::Bytes(2_000_000_000)));
    }

    #[test]
    fn budget_zero_disables() {
        assert_eq!(parse_budget(Some("0")), Ok(Budget::Disabled));
        assert_eq!(parse_budget(Some("0MiB")), Ok(Budget::Disabled));
        assert!(VectorArena::new(Budget::Disabled).is_err());
    }

    // ---- validation: caching behaviour ----

    #[test]
    fn get_caches_and_counts_hits() {
        let arena = arena_4d();
        let calls = Rc::new(Cell::new(0));
        let mut compute = counting_compute(calls.clone());
        let texts = vec!["alpha".to_string(), "beta".to_string()];
        let first = arena.get("ns", 4, &texts, &mut compute).unwrap();
        let second = arena.get("ns", 4, &texts, &mut compute).unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.get(), 1, "second get must be served from the arena");
        let stats = arena.stats().unwrap();
        assert_eq!(stats.pools[0].hits, 2);
        assert_eq!(stats.pools[0].misses, 2);
        assert_eq!(stats.hit_rate, Some(0.5));
    }

    #[test]
    fn generation_namespace_isolation() {
        // Same text under another namespace (e.g. after a head hot-reload)
        // does not match the previous generation's rows.
        let arena = arena_4d();
        let calls = Rc::new(Cell::new(0));
        let mut compute = counting_compute(calls.clone());
        let texts = vec!["alpha".to_string()];
        arena.get("head@0/state", 4, &texts, &mut compute).unwrap();
        arena.get("head@1/state", 4, &texts, &mut compute).unwrap();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let arena = VectorArena::new(Budget::Bytes(64)).expect("tiny arena builds");
        // 64 bytes = 16 cells; 4-wide pool -> 4 rows.
        assert!(arena.reserve(4, 1.0).unwrap());
        let calls = Rc::new(Cell::new(0));
        let mut compute = counting_compute(calls.clone());
        for name in ["a", "b", "c", "d"] {
            arena
                .get("ns", 4, &[name.to_string()], &mut compute)
                .unwrap();
        }
        // Touch "a" so "b" becomes least-recently-used, then add "e".
        arena
            .get("ns", 4, &["a".to_string()], &mut compute)
            .unwrap();
        arena
            .get("ns", 4, &["e".to_string()], &mut compute)
            .unwrap();
        let stats = arena.stats().unwrap();
        assert_eq!(stats.pools[0].evictions, 1);
        // "b" was evicted: asking again recomputes.
        let before = calls.get();
        arena
            .get("ns", 4, &["b".to_string()], &mut compute)
            .unwrap();
        assert_eq!(calls.get(), before + 1);
        // "a" survived: no recompute.
        let before = calls.get();
        arena
            .get("ns", 4, &["a".to_string()], &mut compute)
            .unwrap();
        assert_eq!(calls.get(), before);
    }

    // ---- adversarial: bad budgets and inputs ----

    #[test]
    fn budget_bare_number_at_least_one_rejected() {
        assert_eq!(
            parse_budget(Some("1")),
            Err(BudgetError::FractionOutOfRange)
        );
        assert_eq!(
            parse_budget(Some("2.5")),
            Err(BudgetError::FractionOutOfRange)
        );
    }

    #[test]
    fn budget_unknown_unit_rejected() {
        assert!(matches!(
            parse_budget(Some("10XB")),
            Err(BudgetError::UnknownUnit { .. })
        ));
    }

    #[test]
    fn text_too_long_rejected_before_compute() {
        let arena = arena_4d();
        let calls = Rc::new(Cell::new(0));
        let mut compute = counting_compute(calls.clone());
        let big = "x".repeat(TEXT_BYTES_MAX + 1);
        let err = arena.get("ns", 4, &[big], &mut compute).unwrap_err();
        assert!(matches!(err, CacheError::TextTooLong { .. }));
        assert_eq!(calls.get(), 0, "rejected input must not reach compute");
    }

    #[test]
    fn compute_bad_row_dim_rejected_and_nothing_inserted() {
        let arena = arena_4d();
        let mut bad = |_: &[String]| Ok(vec![vec![1.0, 2.0]]);
        let err = arena
            .get("ns", 4, &["alpha".to_string()], &mut bad)
            .unwrap_err();
        assert!(matches!(err, CacheError::BadVector { index: 0 }));
        let stats = arena.stats().unwrap();
        assert_eq!(
            stats.pools[0].used, 0,
            "failed compute must not publish rows"
        );
        assert_eq!(stats.pools[0].misses, 0);
    }

    #[test]
    fn compute_nonfinite_row_rejected() {
        let arena = arena_4d();
        let mut bad = |_: &[String]| Ok(vec![vec![1.0, f32::NAN, 3.0, 4.0]]);
        let err = arena
            .get("ns", 4, &["alpha".to_string()], &mut bad)
            .unwrap_err();
        assert!(matches!(err, CacheError::BadVector { index: 0 }));
    }

    #[test]
    fn reserve_rejects_bad_share_and_dim() {
        let arena = VectorArena::new(Budget::Bytes(1 << 20)).unwrap();
        assert_eq!(arena.reserve(4, 0.0), Err(CacheError::BadShare));
        assert_eq!(arena.reserve(4, 1.5), Err(CacheError::BadShare));
        assert_eq!(arena.reserve(0, 0.5), Err(CacheError::BadDim { dim: 0 }));
    }

    // ---- validation: direct budget construction ----

    #[test]
    fn budget_direct_fraction_constructs() {
        // A directly constructed in-range fraction is honoured exactly like
        // a parsed one: 0.25 of the 8 GiB device.
        let arena = VectorArena::new(Budget::Fraction(0.25)).unwrap();
        let stats = arena.stats().unwrap();
        let expected_mb = 0.25 * CPU_DEVICE_BYTES as f64 / 1e6;
        assert!(
            (stats.reserved_mb - expected_mb).abs() < 1.0,
            "reserved_mb was {}",
            stats.reserved_mb
        );
    }

    #[test]
    fn concurrent_duplicate_insert_never_orphans_slots() {
        // Eight threads race to insert the same missing key into a 1-row
        // pool. Exactly one slot may be claimed; the losers must reuse the
        // existing slot (touch), never claim a second one for the same key.
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let arena = VectorArena::new(Budget::Bytes(1 << 20)).unwrap();
        assert!(arena.reserve(4, 1.0).unwrap());
        let computed = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let arena = &arena;
                let computed = Arc::clone(&computed);
                scope.spawn(move || {
                    let mut compute = |missing: &[String]| -> Result<Vec<Vec<f32>>, CacheError> {
                        computed.fetch_add(1, Ordering::SeqCst);
                        Ok(missing
                            .iter()
                            .map(|text| {
                                let seed = text.len() as f32;
                                vec![seed, seed + 1.0, seed + 2.0, seed + 3.0]
                            })
                            .collect())
                    };
                    arena
                        .get("ns", 4, &["shared".to_string()], &mut compute)
                        .unwrap();
                });
            }
        });
        let stats = arena.stats().unwrap();
        assert_eq!(stats.pools[0].used, 1, "one key must own exactly one slot");
        assert!(
            computed.load(Ordering::SeqCst) >= 1,
            "at least one thread computed"
        );
    }

    // ---- adversarial: direct budget construction ----

    #[test]
    fn budget_direct_fraction_out_of_range_rejected() {
        for fraction in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.5, 1.0, 2.0] {
            let err = VectorArena::new(Budget::Fraction(fraction))
                .err()
                .expect("must fail");
            assert_eq!(
                err,
                CacheError::BadShare,
                "fraction {fraction} must be rejected"
            );
        }
    }

    #[test]
    fn budget_zero_bytes_is_disabled() {
        let err = VectorArena::new(Budget::Bytes(0)).err().expect("must fail");
        assert_eq!(err, CacheError::Disabled);
        let err = VectorArena::new(Budget::Disabled).err().expect("must fail");
        assert_eq!(err, CacheError::Disabled);
        // -0.0 is zero bytes, not a share violation.
        let err = VectorArena::new(Budget::Fraction(-0.0))
            .err()
            .expect("must fail");
        assert_eq!(err, CacheError::Disabled);
    }
}
