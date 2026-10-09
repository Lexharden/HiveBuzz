//! Persistencia de los jobs pendientes, para no perder acciones si la app se cierra.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::rules::model::{PlanMode, Step};
use crate::rules::template::Vars;

/// Un job = una ejecución de una regla (su plan completo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub rule_id: String,
    pub mode: PlanMode,
    pub steps: Vec<Step>,
    pub vars: Vars,
    pub priority: i32,
    pub created_ms: i64,
    pub expires_ms: i64,
    /// En modo secuencia, índice del próximo paso a ejecutar (para reanudar tras un reinicio).
    pub next_step: usize,
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save(&self, job: &Job) -> Result<()>;
    async fn set_progress(&self, id: &str, next_step: usize) -> Result<()>;
    async fn delete(&self, id: &str) -> Result<()>;
    async fn load_all(&self) -> Result<Vec<Job>>;
}

/// Almacén en memoria (tests y modo sin persistencia).
#[derive(Default)]
pub struct MemoryJobStore {
    jobs: Mutex<HashMap<String, Job>>,
}

impl MemoryJobStore {
    pub fn len(&self) -> usize {
        self.jobs.lock().map_or(0, |m| m.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        self.jobs.lock().ok().and_then(|m| m.get(id).cloned())
    }
}

#[async_trait]
impl JobStore for MemoryJobStore {
    async fn save(&self, job: &Job) -> Result<()> {
        if let Ok(mut m) = self.jobs.lock() {
            m.insert(job.id.clone(), job.clone());
        }
        Ok(())
    }

    async fn set_progress(&self, id: &str, next_step: usize) -> Result<()> {
        if let Ok(mut m) = self.jobs.lock() {
            if let Some(j) = m.get_mut(id) {
                j.next_step = next_step;
            }
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        if let Ok(mut m) = self.jobs.lock() {
            m.remove(id);
        }
        Ok(())
    }

    async fn load_all(&self) -> Result<Vec<Job>> {
        Ok(self.jobs.lock().map_or_else(|_| Vec::new(), |m| m.values().cloned().collect()))
    }
}
