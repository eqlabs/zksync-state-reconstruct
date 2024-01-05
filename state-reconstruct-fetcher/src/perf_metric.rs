use tokio::time::Duration;

pub struct PerfMetric {
    name: String,
    total: Duration,
    count: u32,
}

impl PerfMetric {
    pub fn new(name: &str) -> Self {
        PerfMetric {
            name: String::from(name),
            total: Duration::default(),
            count: 0
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
