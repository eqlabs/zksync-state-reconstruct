use tokio::time::Duration;

#[derive(Default)]
pub struct PerfMetric {
    total: Duration,
    count: u32,
}

impl PerfMetric {
    pub fn add(&mut self, duration: Duration) -> u32 {
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
