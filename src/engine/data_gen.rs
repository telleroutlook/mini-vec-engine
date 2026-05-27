use super::{RecordBatch, BATCH_SIZE};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

pub struct DataGenConfig {
    pub total_rows: usize,
    pub n_distinct_keys: u32,
    pub val_min: i64,
    pub val_max: i64,
    pub seed: u64,
}

impl Default for DataGenConfig {
    fn default() -> Self {
        Self {
            total_rows: 10_000_000,
            n_distinct_keys: 100,
            val_min: 0,
            val_max: 1000,
            seed: 42,
        }
    }
}

pub fn generate_batches(config: &DataGenConfig) -> Vec<RecordBatch> {
    let mut rng = SmallRng::seed_from_u64(config.seed);
    let mut batches = Vec::new();
    let mut rows_generated = 0;

    while rows_generated < config.total_rows {
        let batch_rows = std::cmp::min(BATCH_SIZE, config.total_rows - rows_generated);
        let mut keys = Vec::with_capacity(batch_rows);
        let mut vals = Vec::with_capacity(batch_rows);

        for _ in 0..batch_rows {
            keys.push(rng.random_range(0..config.n_distinct_keys));
            vals.push(rng.random_range(config.val_min..config.val_max));
        }

        batches.push(RecordBatch {
            keys,
            vals,
            num_rows: batch_rows,
        });
        rows_generated += batch_rows;
    }

    batches
}
