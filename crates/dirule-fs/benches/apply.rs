use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dirule_core::{EntryKind, Plan, PlannedEntry};
use dirule_fs::validate_destinations;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

fn benchmark_destination_validation(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("apply_destination_validation");

    for count in [100, 500, 1_000] {
        let target = cycle_fixture(count);
        let mut plan = rename_cycle(count);
        validate_destinations(target.path(), &mut plan).unwrap();
        assert!(plan.is_conflict_free());

        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("existing_destination_cycle", count),
            &(&target, &plan),
            |bencher, (target, plan)| {
                bencher.iter_batched(
                    || (*plan).clone(),
                    |mut plan| {
                        validate_destinations(black_box(target.path()), black_box(&mut plan))
                            .unwrap();
                        black_box(plan)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn cycle_fixture(count: usize) -> TempDir {
    let target = tempfile::tempdir().unwrap();

    for index in 0..count {
        fs::write(target.path().join(format!("file-{index}")), b"").unwrap();
    }

    target
}

fn rename_cycle(count: usize) -> Plan {
    let entries = (0..count)
        .map(|index| PlannedEntry {
            source: PathBuf::from(format!("file-{index}")),
            destination: PathBuf::from(format!("file-{}", (index + 1) % count)),
            kind: EntryKind::File,
            rule: None,
        })
        .collect();

    Plan {
        entries,
        ..Plan::default()
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = benchmark_destination_validation
}
criterion_main!(benches);
