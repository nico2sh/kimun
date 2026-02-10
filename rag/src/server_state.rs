use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::KimunRag;
use crate::config::RagConfig;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub rag: Arc<Mutex<KimunRag>>,
    pub config: Arc<RagConfig>,
    pub job_tracker: Arc<Mutex<JobTracker>>,
}

impl AppState {
    pub fn new(rag: KimunRag, config: RagConfig) -> Self {
        Self {
            rag: Arc::new(Mutex::new(rag)),
            config: Arc::new(config),
            job_tracker: Arc::new(Mutex::new(JobTracker::new())),
        }
    }
}

/// Tracks jobs (queries, indexing operations) with their status
pub struct JobTracker {
    jobs: HashMap<Uuid, Job>,
}

impl JobTracker {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    /// Create a new job
    pub fn create(&mut self, job_id: Uuid, status: JobStatus) -> &Job {
        let job = Job {
            id: job_id,
            status,
            result: None,
            error: None,
            created_at: SystemTime::now(),
        };
        self.jobs.insert(job_id, job);
        self.jobs.get(&job_id).unwrap()
    }

    /// Get a job by ID
    pub fn get(&self, job_id: &Uuid) -> Option<&Job> {
        self.jobs.get(job_id)
    }

    /// Update job status
    pub fn update_status(&mut self, job_id: &Uuid, status: JobStatus) -> Option<()> {
        let job = self.jobs.get_mut(job_id)?;
        job.status = status;
        Some(())
    }

    /// Set job result (marks as completed)
    pub fn set_result(&mut self, job_id: &Uuid, result: String) -> Option<()> {
        let job = self.jobs.get_mut(job_id)?;
        job.status = JobStatus::Completed;
        job.result = Some(result);
        Some(())
    }

    /// Set job error (marks as failed)
    pub fn set_error(&mut self, job_id: &Uuid, error: String) -> Option<()> {
        let job = self.jobs.get_mut(job_id)?;
        job.status = JobStatus::Failed;
        job.error = Some(error);
        Some(())
    }

    /// Clean up old jobs (older than 5 minutes)
    pub fn cleanup_old_jobs(&mut self) {
        let now = SystemTime::now();
        let five_minutes = std::time::Duration::from_secs(300);

        self.jobs.retain(|_, job| {
            if let Ok(elapsed) = now.duration_since(job.created_at) {
                elapsed < five_minutes
            } else {
                true // Keep if we can't determine age
            }
        });
    }
}

/// Represents a job (query or indexing operation)
#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub status: JobStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: SystemTime,
}

/// Job status
#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

impl JobStatus {
    pub fn as_str(&self) -> &str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Processing => "processing",
            JobStatus::Completed => "completed",
            JobStatus::Failed => "failed",
        }
    }
}
