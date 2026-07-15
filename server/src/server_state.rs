use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::RagConfig;
use crate::{KimunRag, RagError};

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    /// The query pipeline, or `None` on an *unconfigured* server (no embedder →
    /// no vector store, adr/0024). Immutable after startup (config edits apply
    /// on restart), so it is shared plainly — no lock.
    pub rag: Option<Arc<KimunRag>>,
    pub config: Arc<RagConfig>,
    /// Where the config was loaded from, so the web UI can persist edits back to
    /// it. `None` disables saving (config supplied without a resolvable path).
    pub config_path: Option<PathBuf>,
    /// Recent WARN/ERROR events, shown on the web UI's Logs page.
    pub log_buffer: crate::logbuffer::LogBuffer,
    /// Why the RAG pipeline failed to build at startup (e.g. the embedding
    /// model download failed). `Some` means the server is running *degraded*:
    /// an embedder is configured but `rag` is `None`, so data endpoints reject
    /// with 503 just like an unconfigured server. Shown on the dashboard.
    pub startup_error: Option<String>,
    pub job_tracker: Arc<Mutex<JobTracker>>,
    /// Serializes index writes (store/delete) so concurrent jobs on the same
    /// collection can't double-insert chunks or race each other's updates.
    /// Queries never take this, so indexing does not block search/answer.
    pub index_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new(rag: Option<KimunRag>, config: RagConfig) -> Self {
        Self {
            rag: rag.map(Arc::new),
            config: Arc::new(config),
            config_path: None,
            log_buffer: crate::logbuffer::LogBuffer::new(),
            startup_error: None,
            job_tracker: Arc::new(Mutex::new(JobTracker::new())),
            index_lock: Arc::new(Mutex::new(())),
        }
    }

    /// The pipeline, or the typed unconfigured rejection — the one gate every
    /// data handler runs before touching a request (adr/0024).
    pub fn rag(&self) -> Result<&Arc<KimunRag>, RagError> {
        self.rag.as_ref().ok_or(RagError::Unconfigured)
    }

    /// Records the on-disk config path so the web UI can write edits back to it.
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = Some(path);
        self
    }

    /// Attaches the log buffer the tracing layer writes to, so the web UI can
    /// render recent warnings/errors.
    pub fn with_log_buffer(mut self, buffer: crate::logbuffer::LogBuffer) -> Self {
        self.log_buffer = buffer;
        self
    }

    /// Records why RAG initialization failed (degraded startup, adr/0024's
    /// unconfigured behavior with a visible cause).
    pub fn with_startup_error(mut self, error: Option<String>) -> Self {
        self.startup_error = error;
        self
    }
}

/// Tracks jobs (queries, indexing operations) with their status
pub struct JobTracker {
    jobs: HashMap<Uuid, Job>,
}

impl Default for JobTracker {
    fn default() -> Self {
        Self::new()
    }
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

    /// All tracked jobs, newest first — for the web UI's job list.
    pub fn list(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = self.jobs.values().cloned().collect();
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        jobs
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

    /// Evict jobs older than the retention window. Kept above the client's
    /// answer-poll ceiling (~12 min, `ANSWER_POLL_ATTEMPTS` in
    /// `kimun_server_client`) so a slow-but-live job is never deleted out
    /// from under a polling client; also bounds a job that a panicking task
    /// left stuck in `Processing`.
    pub fn cleanup_old_jobs(&mut self) {
        let now = SystemTime::now();
        let retention = std::time::Duration::from_secs(15 * 60);

        self.jobs.retain(|_, job| {
            if let Ok(elapsed) = now.duration_since(job.created_at) {
                elapsed < retention
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
