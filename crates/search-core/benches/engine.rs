//! Search latency on a synthetic vault.
//!
//! `cargo bench -p search-core --bench engine [-- <filter>]`
//!
//! The vault is generated deterministically (fixed xorshift seed, same note text as
//! `crates/app/examples/perf_budget.rs`) and cached in the system temp dir, so repeated runs measure
//! the same bytes. The crate under test is only `search-core` (plus `vault-core` for the index), so
//! this bench never depends on `app`.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use search_core::{Corpus, Query, RankWeights, Sort};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vault_core::VaultStore;

/// Notes in the synthetic vault. Sized so `line:(paragraph 3 alias)` lands at a few ms per
/// iteration: the same query on the 50,000-note perf_budget vault takes ~22 ms.
const NOTES: usize = 5_000;

/// Bump when the generator changes so a stale cached vault is never reused.
const VAULT_VERSION: &str = "v1";

/// The query the dream-rsi loop scores, plus one full-text and one tag query for context.
const QUERIES: [&str; 3] = ["line:(paragraph 3 alias)", "filler text", "tag:#tag7 realistic"];

/// Deterministic synthetic vault: 20 areas x 25 topics, six paragraphs per note.
fn generate(root: &Path, n: usize) {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = move |m: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % m as u64) as usize
    };
    for i in 0..n {
        let dir = root.join(format!("area{:02}/topic{:02}", i % 20, (i / 20) % 25));
        fs::create_dir_all(&dir).unwrap();
        let mut t =
            format!("---\ntags: [area{}, kind/{}]\naliases: [N{i}]\n---\n# Note {i}\n\n", i % 20, i % 7);
        for p in 0..6 {
            t.push_str(&format!(
                "Paragraph {p} of note {i} linking to [[Note {}]] and [[Note {}|alias]] with #tag{} and some filler text to make the note a realistic size for indexing benchmarks.\n\n",
                rnd(n), rnd(n), rnd(50)
            ));
        }
        t.push_str("## Tasks\n\n- [ ] open task\n- [x] done task ^blk\n");
        fs::write(dir.join(format!("Note {i}.md")), t).unwrap();
    }
}

/// Path of the cached vault, generating it on first use. A `.complete` marker (dot-files are
/// skipped by the vault walk) tells a finished generation from an interrupted one.
fn vault_root(n: usize) -> PathBuf {
    let root = std::env::temp_dir().join(format!("metamind-bench-engine-{VAULT_VERSION}-{n}"));
    if !root.join(".complete").exists() {
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        generate(&root, n);
        fs::write(root.join(".complete"), format!("{n} notes\n")).unwrap();
    }
    root
}

struct Fixture {
    root: PathBuf,
    store: VaultStore,
    corpus: Corpus,
}

impl Fixture {
    fn new(n: usize) -> Fixture {
        let root = vault_root(n);
        let (store, _) = VaultStore::open(&root).unwrap();
        let corpus = Corpus::build(&store.index, &root);
        eprintln!(
            "bench vault: {} notes, {:.1} MB lowercased, at {}",
            n,
            corpus.bytes() as f64 / 1e6,
            root.display()
        );
        Fixture { root, store, corpus }
    }

    fn hits(&self, query: &Query) -> Vec<search_core::Hit> {
        self.corpus.search(&self.store.index, &self.root, query, Sort::Relevance, RankWeights::default(), 50)
    }
}

fn benches(c: &mut Criterion) {
    let fx = Fixture::new(NOTES);
    let mut group = c.benchmark_group("search");
    group.throughput(Throughput::Elements(NOTES as u64));
    // Short enough to keep the whole group under ~10 s, long enough for a stable median.
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(2));
    // At ~2.5 ms per iteration, 50 samples need >3 s of wall clock regardless of `measurement_time`.
    // 30 samples keeps the whole group at ~9 s while still averaging ~930 iterations per bench.
    group.sample_size(30);
    for id in QUERIES {
        let query = Query::parse(id).unwrap();
        assert!(!fx.hits(&query).is_empty(), "query `{id}` matches nothing in the bench vault");
        group.bench_function(id, |b| b.iter(|| fx.hits(&query)));
    }
    group.finish();
}

criterion_group!(engine, benches);
criterion_main!(engine);
