use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dirule_core::EntryKind;
use dirule_fs::scan;
use std::fs;
use std::hint::black_box;
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

fn fixture(visible_files: usize) -> TempDir {
    let target = tempfile::tempdir().unwrap();
    let source = target.path().join("src");
    let module_count = visible_files.div_ceil(100);

    fs::write(target.path().join(".ignore"), "ignored/\n").unwrap();
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

    let ignored_directory = target.path().join("ignored");
    fs::create_dir_all(&ignored_directory).unwrap();
    for index in 0..visible_files.div_ceil(10) {
        fs::write(ignored_directory.join(format!("generated-{index}.rs")), b"").unwrap();
    }

    target
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = benchmark_scan
}
criterion_main!(benches);
