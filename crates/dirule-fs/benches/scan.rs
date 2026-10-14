use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dirule_core::{EntryKind, Matcher, MatcherSet};
use dirule_fs::{scan, scan_with_ignore_set};
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

fn benchmark_scan(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("scan");

    for count in [100, 1_000, 10_000, 100_000] {
        let target = fixture(count);
        let fixture_scan = scan(target.path()).unwrap();
        let visible_files = fixture_scan
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && entry.path.starts_with("src"))
            .count();

        assert_eq!(visible_files, count);
        assert!(
            fixture_scan
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("ignored"))
        );

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("visible_files_with_ignored_tree", count),
            &target,
            |bencher, target| bencher.iter(|| black_box(scan(black_box(target.path())).unwrap())),
        );
    }

    group.finish();
}

fn benchmark_configured_ignore_overhead(criterion: &mut Criterion) {
    let visible_files = 10_000;
    let target = configured_ignore_fixture(visible_files);
    fs::write(target.path().join(".ignore"), "generated/\n").unwrap();
    let baseline = scan_with_ignore_set(target.path(), &MatcherSet::default()).unwrap();

    assert_eq!(
        baseline
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && entry.path.starts_with("src"))
            .count(),
        visible_files
    );

    let mut group = criterion.benchmark_group("scan/configured_ignore_overhead");
    group.measurement_time(Duration::from_secs(1));
    group.sample_size(10);

    for count in [0, 1, 4, 16] {
        let ignores = MatcherSet::new(
            (0..count).map(|index| Matcher::new(&format!("not-present-{index}/**")).unwrap()),
        );
        let result = scan_with_ignore_set(target.path(), &ignores).unwrap();
        let files = result
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && entry.path.starts_with("src"))
            .count();

        assert_eq!(files, visible_files);

        group.bench_with_input(
            BenchmarkId::new("patterns", count),
            &ignores,
            |bencher, ignores| {
                bencher.iter(|| {
                    black_box(
                        scan_with_ignore_set(black_box(target.path()), black_box(ignores)).unwrap(),
                    )
                })
            },
        );
    }

    group.finish();
}

fn benchmark_configured_ignore_pruning(criterion: &mut Criterion) {
    let visible_files = 10_000;
    let generated_files = visible_files / 10;
    let target = configured_ignore_fixture(visible_files);
    let baseline = scan(target.path()).unwrap();

    assert_eq!(
        baseline
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File)
            .count(),
        visible_files + generated_files
    );

    let mut group = criterion.benchmark_group("scan/configured_ignore_pruning");
    group.measurement_time(Duration::from_secs(1));
    group.sample_size(10);

    group.bench_function("no_configured_ignore", |bencher| {
        bencher.iter(|| black_box(scan(black_box(target.path())).unwrap()))
    });

    for count in [1, 4, 16] {
        let ignores = MatcherSet::new(
            (0..count)
                .map(|index| Matcher::new(&format!("not-present-{index}/**")).unwrap())
                .chain([Matcher::new("generated/**").unwrap()]),
        );
        let result = scan_with_ignore_set(target.path(), &ignores).unwrap();
        let files = result
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::File && entry.path.starts_with("src"))
            .count();

        assert_eq!(files, visible_files);
        assert!(
            result
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with("generated"))
        );

        group.bench_with_input(
            BenchmarkId::new("patterns_before_match", count + 1),
            &ignores,
            |bencher, ignores| {
                bencher.iter(|| {
                    black_box(
                        scan_with_ignore_set(black_box(target.path()), black_box(ignores)).unwrap(),
                    )
                })
            },
        );
    }

    group.finish();
}

fn fixture(visible_files: usize) -> TempDir {
    let target = tempfile::tempdir().unwrap();
    fs::write(target.path().join(".ignore"), "ignored/\n").unwrap();
    source_tree(target.path(), visible_files);

    let ignored_directory = target.path().join("ignored");
    fs::create_dir_all(&ignored_directory).unwrap();
    for index in 0..visible_files.div_ceil(10) {
        fs::write(ignored_directory.join(format!("generated-{index}.rs")), b"").unwrap();
    }

    target
}

fn configured_ignore_fixture(visible_files: usize) -> TempDir {
    let target = tempfile::tempdir().unwrap();
    source_tree(target.path(), visible_files);

    let generated = target.path().join("generated");
    fs::create_dir_all(&generated).unwrap();
    for index in 0..visible_files / 10 {
        fs::write(generated.join(format!("Client-{index}.ts")), b"").unwrap();
    }

    target
}

fn source_tree(target: &Path, visible_files: usize) {
    let source = target.join("src");
    let module_count = visible_files.div_ceil(100);
    fs::create_dir_all(&source).unwrap();

    for module in 0..module_count {
        fs::create_dir_all(source.join(format!("Module-{module}"))).unwrap();
    }

    for index in 0..visible_files {
        let path = source
            .join(format!("Module-{}", index / 100))
            .join(format!("MyXMLParser-{index}.rs"));
        fs::write(path, b"").unwrap();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = benchmark_scan, benchmark_configured_ignore_overhead,
        benchmark_configured_ignore_pruning
}
criterion_main!(benches);
