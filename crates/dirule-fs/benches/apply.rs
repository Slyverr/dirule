use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dirule_core::{EntryKind, Plan, PlannedEntry};
use dirule_fs::{
    ConflictPolicy, JournalStore, apply, apply_journaled, apply_with_conflict_policy, scan,
    validate_destinations,
};
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

fn benchmark_file_rename_cycles(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("apply_file_rename_cycle");

    for count in [100, 500, 1_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(
            BenchmarkId::new("existing_destinations", count),
            |bencher| {
                bencher.iter_batched(
                    || {
                        let target = cycle_fixture(count);
                        let snapshot = scan(target.path()).unwrap();
                        let plan = rename_cycle(count);

                        (target, snapshot, plan)
                    },
                    |(target, snapshot, plan)| {
                        black_box(apply(target.path(), &plan, &snapshot).unwrap());
                        target
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn benchmark_skip_conflicts(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("apply_skip_conflicts");

    for count in [100, 500, 1_000] {
        group.throughput(Throughput::Elements((count * 2) as u64));
        group.bench_with_input(
            BenchmarkId::new("conflicting_and_independent_moves", count),
            &count,
            |bencher, &count| {
                bencher.iter_batched(
                    || {
                        let target = tempfile::tempdir().unwrap();
                        let mut entries = Vec::with_capacity(count * 2);
                        let mut conflicts = Vec::with_capacity(count);

                        for index in 0..count {
                            let blocked_source = format!("blocked-source-{index}");
                            let blocked_destination = format!("blocked-destination-{index}");
                            let independent_source = format!("independent-source-{index}");
                            let independent_destination =
                                format!("independent-destination-{index}");

                            fs::write(target.path().join(&blocked_source), b"blocked").unwrap();
                            fs::write(target.path().join(&independent_source), b"independent")
                                .unwrap();
                            entries.push(PlannedEntry {
                                source: PathBuf::from(blocked_source),
                                destination: PathBuf::from(&blocked_destination),
                                kind: EntryKind::File,
                                rule: None,
                            });
                            entries.push(PlannedEntry {
                                source: PathBuf::from(independent_source),
                                destination: PathBuf::from(independent_destination),
                                kind: EntryKind::File,
                                rule: None,
                            });
                            conflicts.push(dirule_core::Conflict {
                                path: PathBuf::from(blocked_destination),
                                reason: "benchmark conflict".into(),
                            });
                        }

                        let snapshot = scan(target.path()).unwrap();
                        let plan = Plan {
                            entries,
                            conflicts,
                            ..Plan::default()
                        };

                        (target, snapshot, plan)
                    },
                    |(target, snapshot, plan)| {
                        black_box(
                            apply_with_conflict_policy(
                                target.path(),
                                &plan,
                                &snapshot,
                                ConflictPolicy::Skip,
                            )
                            .unwrap(),
                        );
                        target
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn benchmark_journaled_file_rename_cycles(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("apply_journaled_file_rename_cycle");

    for count in [100, 500, 1_000] {
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(
            BenchmarkId::new("existing_destinations", count),
            |bencher| {
                bencher.iter_batched(
                    || {
                        let state = tempfile::tempdir().unwrap();
                        let target = state.path().join("target");
                        fs::create_dir(&target).unwrap();
                        for index in 0..count {
                            fs::write(target.join(format!("file-{index}")), b"").unwrap();
                        }
                        let snapshot = scan(&target).unwrap();
                        let plan = rename_cycle(count);
                        let store = JournalStore::new(state.path().join("state/runs"));

                        (state, target, snapshot, plan, store)
                    },
                    |(state, target, snapshot, plan, store)| {
                        black_box(
                            apply_journaled(
                                &target,
                                &plan,
                                &snapshot,
                                ConflictPolicy::Abort,
                                &store,
                            )
                            .unwrap(),
                        );
                        state
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
    targets = benchmark_destination_validation,
        benchmark_file_rename_cycles,
        benchmark_skip_conflicts,
        benchmark_journaled_file_rename_cycles
}
criterion_main!(benches);
