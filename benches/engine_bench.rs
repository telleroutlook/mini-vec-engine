use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mini_vec_engine::engine::{self, data_gen, QueryParams};

fn bench_engines(c: &mut Criterion) {
    let total_rows = 1_000_000;
    let config = data_gen::DataGenConfig {
        total_rows,
        n_distinct_keys: 100,
        val_min: 0,
        val_max: 1000,
        seed: 42,
    };
    let batches = data_gen::generate_batches(&config);
    let params = QueryParams { threshold: 500 };

    let mut group = c.benchmark_group("query_engines");
    group.throughput(Throughput::Elements(total_rows as u64));
    group.sample_size(20);

    group.bench_function("naive", |b| {
        b.iter(|| engine::naive::execute(&batches, &params))
    });

    group.bench_function("vectorized_early", |b| {
        b.iter(|| engine::vectorized::execute_early(&batches, &params))
    });

    group.bench_function("vectorized_late", |b| {
        b.iter(|| engine::vectorized::execute_late(&batches, &params))
    });

    group.bench_function("parallel", |b| {
        b.iter(|| engine::parallel::execute(&batches, &params))
    });

    group.finish();
}

fn bench_selectivity(c: &mut Criterion) {
    let total_rows = 1_000_000;
    let config = data_gen::DataGenConfig {
        total_rows,
        n_distinct_keys: 100,
        val_min: 0,
        val_max: 1000,
        seed: 42,
    };
    let batches = data_gen::generate_batches(&config);

    let mut group = c.benchmark_group("selectivity_late_materialization");
    group.throughput(Throughput::Elements(total_rows as u64));
    group.sample_size(20);

    for threshold in [100i64, 500, 900] {
        let params = QueryParams { threshold };
        group.bench_with_input(
            BenchmarkId::new("threshold", threshold),
            &params,
            |b, params| b.iter(|| engine::vectorized::execute_late(&batches, params)),
        );
    }

    group.finish();
}

criterion_group!(benches, bench_engines, bench_selectivity);
criterion_main!(benches);
