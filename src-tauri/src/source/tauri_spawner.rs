//! Lanzador real del sidecar mediante `tauri-plugin-shell` (resuelve el `externalBin`
//! tanto en desarrollo como en el instalador).

use tauri::AppHandle;
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tokio::sync::{mpsc, oneshot};

use super::sidecar::{ProcessHandle, ProcessOutput, Spawner};
use crate::error::{AppError, Result};

/// Nombre del binario declarado en `bundle.externalBin` (sin ruta ni triple).
const SIDECAR_NAME: &str = "tiktok-sidecar";

pub struct TauriSpawner {
    app: AppHandle,
}

impl TauriSpawner {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Spawner for TauriSpawner {
    fn spawn(&self) -> Result<ProcessHandle> {
        let command = self
            .app
            .shell()
            .sidecar(SIDECAR_NAME)
            .map_err(|e| AppError::Sidecar(e.to_string()))?;
        let (mut events, mut child) = command
            .spawn()
            .map_err(|e| AppError::Sidecar(e.to_string()))?;

        let (out_tx, out_rx) = mpsc::channel::<ProcessOutput>(1024);
        let (in_tx, mut in_rx) = mpsc::channel::<String>(64);
        let (kill_tx, mut kill_rx) = oneshot::channel::<()>();

        // Salida del proceso → canal. Un chunk puede traer varias líneas.
        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                let item = match event {
                    CommandEvent::Stdout(bytes) => {
                        let text = String::from_utf8_lossy(&bytes).into_owned();
                        for line in text.lines().filter(|l| !l.trim().is_empty()) {
                            if out_tx
                                .send(ProcessOutput::Stdout(line.to_string()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        continue;
                    }
                    CommandEvent::Stderr(bytes) => {
                        ProcessOutput::Stderr(String::from_utf8_lossy(&bytes).trim_end().to_string())
                    }
                    CommandEvent::Error(e) => ProcessOutput::Stderr(format!("error de E/S: {e}")),
                    CommandEvent::Terminated(payload) => ProcessOutput::Exited(payload.code),
                    _ => continue,
                };
                let exited = matches!(item, ProcessOutput::Exited(_));
                if out_tx.send(item).await.is_err() || exited {
                    return;
                }
            }
            let _ = out_tx.send(ProcessOutput::Exited(None)).await;
        });

        // Escritura de comandos y muerte del proceso: un único dueño del `child`.
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::select! {
                    line = in_rx.recv() => match line {
                        Some(line) => {
                            if let Err(e) = child.write(line.as_bytes()) {
                                tracing::warn!(error = %e, "no se pudo escribir al sidecar");
                            }
                        }
                        None => break,
                    },
                    _ = &mut kill_rx => break,
                }
            }
            let _ = child.kill();
        });

        Ok(ProcessHandle {
            output: out_rx,
            stdin: in_tx,
            kill: Box::new(move || {
                let _ = kill_tx.send(());
            }),
        })
    }
}
