use tokio::time::Duration;

pub const METRICS_TRACING_TARGET: &str = "metrics";

/// Average, explicitly resettable time used by a specific (named) operation.
pub struct PerfMetric {
    name: String,
    total: Duration,
    count: u32,
}

pub struct L1Metrics {
    // Metrics variables.
    pub l1_blocks_processed: u64,
    pub l2_blocks_processed: u64,
    pub latest_l1_block_nbr: u64,
    pub latest_l2_block_nbr: u64,

    pub first_l1_block: u64,
    pub last_l1_block: u64,

    /// Time taken to procure a log from L1.
    pub log_acquisition: PerfMetric,

    /// Time taken to procure a transaction from L1.
    pub tx_acquisition: PerfMetric,

    /// Time taken to parse a [`CommitBlockInfo`] from a transaction.
    pub parsing: PerfMetric,
}

impl PerfMetric {
    pub fn new(name: &str) -> Self {
        PerfMetric {
            name: String::from(name),
            total: Duration::default(),
            count: 0,
        }
    }

    pub fn add(&mut self, duration: Duration) -> u32 {
        tracing::trace!("{}: {:?}", self.name, duration);
        self.total += duration;
        self.count += 1;
        self.count
    }

    pub fn renew(&mut self) -> String {
        let old = self.format();
        self.total = Duration::default();
        self.count = 0;
        old
    }

    pub fn format(&self) -> String {
        if self.count == 0 {
            String::from("-")
        } else {
            let duration = self.total / self.count;
            format!("{:?}", duration)
        }
    }
}

impl Default for L1Metrics {
    fn default() -> Self {
        L1Metrics {
            l1_blocks_processed: 0,
            l2_blocks_processed: 0,
            latest_l1_block_nbr: 0,
            latest_l2_block_nbr: 0,
            first_l1_block: 0,
            last_l1_block: 0,
            log_acquisition: PerfMetric::new("log_acquisition"),
            tx_acquisition: PerfMetric::new("tx_acquisition"),
            parsing: PerfMetric::new("parsing"),
        }
    }
}

impl L1Metrics {
    pub fn print(&mut self) {
        if self.latest_l1_block_nbr == 0 {
            return;
        }

        let progress = {
            let total = self.last_l1_block - self.first_l1_block;
            let cur = self.latest_l1_block_nbr - self.first_l1_block;
            // If polling past `last_l1_block`, stop at 100%.
            let perc = std::cmp::min((cur * 100) / total, 100);
            format!("{perc:>2}%")
        };

        let log_acquisition = self.log_acquisition.renew();
        let tx_acquisition = self.tx_acquisition.renew();
        let parsing = self.parsing.renew();

        tracing::info!(
            "PROGRESS: [{}] CUR BLOCK L1: {} L2: {} TOTAL BLOCKS PROCESSED L1: {} L2: {}",
            progress,
            self.latest_l1_block_nbr,
            self.latest_l2_block_nbr,
            self.l1_blocks_processed,
            self.l2_blocks_processed
        );
        tracing::debug!(
            target: METRICS_TRACING_TARGET,
            "ACQUISITION: log {} tx {} parse {}",
            log_acquisition,
            tx_acquisition,
            parsing
        );
    }
}
