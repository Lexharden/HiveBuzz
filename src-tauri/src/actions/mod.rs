//! Acciones: ejecutores enchufables y la cola que los alimenta.
//!
//! Añadir un ejecutor = implementar `ActionExecutor` y registrarlo; el núcleo no cambia.

pub mod clock;
pub mod queue;
pub mod store;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::error::{AppError, Result};
use crate::rules::template::{render, Vars};

/// Contexto con el que se ejecuta una acción. Todo lo que sabe del evento original viaja
/// en `vars` (así el job se puede persistir y reanudar tras un reinicio).
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub rule_id: String,
    /// Variables para las plantillas (`{nickname}`, `{coins}`…).
    pub vars: Vars,
}

impl ActionContext {
    /// Aplica las variables a una plantilla de texto.
    pub fn render(&self, template: &str) -> String {
        render(template, &self.vars)
    }
}

/// Cuántas acciones de un tipo pueden correr a la vez.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Concurrency {
    /// Sin restricción propia (solo el tope global).
    Parallel,
    /// Una a la vez dentro del grupo nombrado (p. ej. un TTS a la vez).
    Serial(&'static str),
}

#[async_trait]
pub trait ActionExecutor: Send + Sync {
    /// Valor de `"type"` que este ejecutor atiende.
    fn kind(&self) -> &'static str;

    fn concurrency(&self) -> Concurrency {
        Concurrency::Parallel
    }

    /// Comprueba los parámetros al guardar la regla (no al ejecutarla).
    fn validate(&self, _params: &Map<String, Value>) -> Result<()> {
        Ok(())
    }

    async fn execute(&self, ctx: &ActionContext, params: &Map<String, Value>) -> Result<()>;
}

/// Ejecutores disponibles, por tipo.
#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    map: HashMap<String, Arc<dyn ActionExecutor>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, executor: Arc<dyn ActionExecutor>) {
        self.map.insert(executor.kind().to_string(), executor);
    }

    pub fn get(&self, kind: &str) -> Option<&Arc<dyn ActionExecutor>> {
        self.map.get(kind)
    }

    pub fn kinds(&self) -> Vec<&str> {
        let mut k: Vec<&str> = self.map.keys().map(String::as_str).collect();
        k.sort_unstable();
        k
    }

    /// Valida cada acción de una regla contra su ejecutor.
    pub fn validate_rule(&self, rule: &crate::rules::model::Rule) -> Result<()> {
        for (i, step) in rule.plan.steps.iter().enumerate() {
            let exec = self.get(&step.action.kind).ok_or_else(|| {
                AppError::Invalid(format!(
                    "regla «{}»: la acción {} usa un tipo desconocido («{}»)",
                    rule.name,
                    i + 1,
                    step.action.kind
                ))
            })?;
            exec.validate(&step.action.params).map_err(|e| {
                AppError::Invalid(format!("regla «{}», acción {}: {e}", rule.name, i + 1))
            })?;
        }
        Ok(())
    }
}
