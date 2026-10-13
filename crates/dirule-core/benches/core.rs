use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use dirule_core::{
    AcronymLexicon, AcronymOptions, Config, Entry, EntryKind, Matcher, Rule, RuleKind,
    parse_config, plan, plan_compiled, transform_filename,
};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

const CONFIG_SOURCE: &str = r#"
version = "1.2"
target = "."

[acronyms]
entries = ["API", "HTTP", "XML"]

[[rules]]
name = "Normalize invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf"

[[rules]]
name = "Normalize other names"
match = "**/*"
to = "builtin:kebab-case"
"#;

fn benchmark_matching(criterion: &mut Criterion) {
    let matcher = Matcher::new("Invoices/<year>-<vendor>-<title>.pdf").unwrap();
    let matching_path = "Invoices/2026-Acme-Quarter-1.pdf";
    let non_matching_path = "Invoices/2026-Acme-Quarter-1.txt";

    let mut group = criterion.benchmark_group("matching");
    group.bench_function("named_capture_match", |bencher| {
        bencher.iter(|| matcher.captures(black_box(matching_path)))
    });
    group.bench_function("named_capture_miss", |bencher| {
        bencher.iter(|| matcher.captures(black_box(non_matching_path)))
    });
    group.finish();
}

fn benchmark_transform(criterion: &mut Criterion) {
    let acronyms = AcronymLexicon::default();

    criterion.bench_function("transform/filename_kebab_case", |bencher| {
        bencher.iter(|| {
            transform_filename(
                black_box("MyXMLParser_Q4.Report.pdf"),
                dirule_core::Transform::KebabCase,
                &acronyms,
            )
        })
    });

    criterion.bench_function("transform/filename_pascal_case", |bencher| {
        bencher.iter(|| {
            transform_filename(
                black_box("MyXMLParser_Q4.Report.pdf"),
                dirule_core::Transform::PascalCase,
                &acronyms,
            )
        })
    });
}

fn benchmark_config(criterion: &mut Criterion) {
    criterion.bench_function("config/parse_and_validate", |bencher| {
        bencher.iter(|| parse_config(black_box(CONFIG_SOURCE)).unwrap())
    });
}

fn benchmark_planning(criterion: &mut Criterion) {
    let config = Config {
        name: Some("Benchmark".into()),
        target: Some(PathBuf::from(".")),
        acronyms: AcronymOptions {
            entries: vec!["API".into(), "HTTP".into(), "XML".into()],
        },
        rules: vec![
            Rule {
                name: "Normalize invoices".into(),
                pattern: "Invoices/<year>-<vendor>-<title>.pdf".into(),
                kind: RuleKind::File,
                exclude: Vec::new(),
                to: "template:Archive/${year}/${vendor|kebab-case}/${title|kebab-case}.pdf".into(),
            },
            Rule {
                name: "Normalize other names".into(),
                pattern: "**/*".into(),
                kind: RuleKind::Any,
                exclude: Vec::new(),
                to: "builtin:kebab-case".into(),
            },
        ],
    };
    let compiled = config.compile().unwrap();
    let mut group = criterion.benchmark_group("planning");

    for count in [100, 1_000, 10_000, 100_000] {
        let entries = entries(count);
        group.throughput(Throughput::Elements(count as u64));

        group.bench_with_input(
            BenchmarkId::new("compile_each_run", count),
            &entries,
            |bencher, entries| {
                bencher.iter(|| plan(black_box(&config), black_box(entries)).unwrap())
            },
        );
        group.bench_with_input(
            BenchmarkId::new("reuse_compiled_config", count),
            &entries,
            |bencher, entries| {
                bencher.iter(|| plan_compiled(black_box(&compiled), black_box(entries)).unwrap())
            },
        );
    }

    group.finish();
}

fn entries(count: usize) -> Vec<Entry> {
    (0..count)
        .map(|index| match index % 20 {
            0 => Entry {
                path: PathBuf::from(format!("Invoices/2026-Acme-Quarter-{index}.pdf")),
                kind: EntryKind::File,
            },
            _ if index % 10 == 0 => Entry {
                path: PathBuf::from(format!("src/Module-{index}")),
                kind: EntryKind::Directory,
            },
            _ => Entry {
                path: PathBuf::from(format!(
                    "src/Module-{}/MyXMLParser-{index}.rs",
                    index / 10 * 10
                )),
                kind: EntryKind::File,
            },
        })
        .collect()
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(20);
    targets = benchmark_matching, benchmark_transform, benchmark_config, benchmark_planning
}
criterion_main!(benches);
