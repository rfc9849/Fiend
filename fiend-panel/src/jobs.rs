use std::collections::HashMap;
use std::sync::Mutex;

use crate::App;
use crate::types::{JobStep, JobView};

#[derive(Default)]
pub struct Jobs {
    jobs: Mutex<HashMap<String, JobView>>,
}

impl Jobs {
    pub fn create(&self, kind: &str, target: &str, steps: &[&str]) -> String {
        let id = hex::encode(rand::random::<[u8; 12]>());
        let job = JobView {
            id: id.clone(),
            kind: kind.into(),
            target: target.into(),
            steps: steps
                .iter()
                .map(|name| JobStep {
                    name: (*name).into(),
                    state: "pending".into(),
                    detail: None,
                })
                .collect(),
            done: false,
            error: None,
            node_id: None,
        };
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        if jobs.len() > 64 {
            jobs.retain(|_, job| !job.done);
        }
        jobs.insert(id.clone(), job);
        id
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        self.jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
    }

    pub fn list(&self) -> Vec<JobView> {
        let mut jobs: Vec<_> = self
            .jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        jobs.sort_by(|left, right| left.id.cmp(&right.id));
        jobs
    }

    fn change(&self, id: &str, change: impl FnOnce(&mut JobView)) {
        if let Some(job) = self
            .jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(id)
        {
            change(job);
        }
    }
}

pub struct Progress {
    app: App,
    id: String,
}

impl Progress {
    pub fn new(app: &App, id: &str) -> Self {
        Self {
            app: app.clone(),
            id: id.into(),
        }
    }

    fn emit(&self) {
        self.app.emit("job", Some(self.id.clone()));
    }

    pub fn step(&self, index: usize, state: &str, detail: Option<String>) {
        self.app.jobs.change(&self.id, |job| {
            if let Some(step) = job.steps.get_mut(index) {
                step.state = state.into();
                step.detail = detail;
            }
        });
        self.emit();
    }

    pub fn node(&self, node: i64) {
        self.app
            .jobs
            .change(&self.id, |job| job.node_id = Some(node));
    }

    pub fn finish(&self, error: Option<String>) {
        self.app.jobs.change(&self.id, |job| {
            job.done = true;
            if let Some(error) = &error {
                for step in &mut job.steps {
                    if step.state == "running" {
                        step.state = "failed".into();
                        step.detail.get_or_insert_with(|| error.clone());
                    }
                }
            }
            job.error = error;
        });
        self.emit();
    }

    pub async fn run<T, F>(&self, index: usize, operation: F) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T, String>>,
    {
        self.step(index, "running", None);
        match operation.await {
            Ok(value) => {
                self.step(index, "done", None);
                Ok(value)
            }
            Err(error) => {
                self.step(index, "failed", Some(error.clone()));
                Err(error)
            }
        }
    }
}
