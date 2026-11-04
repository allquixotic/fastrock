//! Windows relaunches activate the process that owns this profile. A named
//! pipe is both the lifetime lock and a bounded, local activation channel.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
};

const LIMIT: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Launch {
    pub folder: Option<PathBuf>,
    pub resume: Option<String>,
}
pub(crate) struct Owner {
    pipe: Option<NamedPipeServer>,
    name: String,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
fn pipe(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    ServerOptions::new()
        .first_pipe_instance(first)
        .reject_remote_clients(true)
        .create(name)
}
pub(crate) fn acquire(cli: &crate::Cli, rt: &tokio::runtime::Handle) -> Result<Option<Owner>> {
    let root = goose_runtime::config::paths::Paths::data_dir();
    let key = root.to_string_lossy().replace('/', "\\").to_lowercase();
    let name = format!(
        r"\\.\pipe\fastrock-profile-{:x}",
        Sha256::digest(key.as_bytes())
    );
    let launch = Launch {
        folder: cli
            .folder
            .as_ref()
            .map(|p| {
                if p.is_absolute() {
                    Ok(p.clone())
                } else {
                    std::env::current_dir().map(|cwd| cwd.join(p))
                }
            })
            .transpose()?,
        resume: cli.resume.clone(),
    };
    rt.block_on(acquire_at(name, launch))
}
async fn acquire_at(name: String, launch: Launch) -> Result<Option<Owner>> {
    match pipe(&name, true) {
        Ok(pipe) => Ok(Some(Owner {
            pipe: Some(pipe),
            name,
            task: None,
        })),
        Err(error) if error.raw_os_error() == Some(5) => {
            tokio::time::timeout(Duration::from_secs(10), forward(&name, &launch))
                .await.context("Fastrock is already running but has not responded. Use its tray window, or Quit that instance before reopening")??;
            Ok(None)
        }
        Err(error) => Err(error).context("Could not establish Fastrock profile ownership"),
    }
}
async fn forward(name: &str, launch: &Launch) -> Result<()> {
    let mut client = loop {
        match ClientOptions::new().open(name) {
            Ok(client) => break client,
            Err(error) if matches!(error.raw_os_error(), Some(2 | 231)) => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(error) => return Err(error.into()),
        }
    };
    let pid = client.read_u32_le().await?;
    // The user launched this process. Transfer its foreground permission to
    // the existing window before asking that process to restore from the tray.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
    let bytes = serde_json::to_vec(launch)?;
    ensure!(bytes.len() <= LIMIT, "Activation request exceeds 64 KiB");
    client.write_u32_le(bytes.len() as u32).await?;
    client.write_all(&bytes).await?;
    ensure!(
        client.read_u8().await? == 1,
        "Existing Fastrock instance could not accept activation"
    );
    Ok(())
}
impl Owner {
    pub fn listen(
        &mut self,
        rt: &tokio::runtime::Handle,
        accept: impl Fn(Launch) -> Result<()> + Send + Sync + 'static,
    ) {
        let mut listener = self.pipe.take().expect("activation listener starts once");
        let name = self.name.clone();
        self.task = Some(rt.spawn(async move {
            loop {
                if listener.connect().await.is_err() {
                    break;
                }
                // Keep an open server handle throughout the handoff, so a
                // third launch can never acquire first-instance ownership.
                let next = match pipe(&name, false) {
                    Ok(next) => next,
                    Err(_) => break,
                };
                let mut client = std::mem::replace(&mut listener, next);
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    client.write_u32_le(std::process::id()).await?;
                    let len = client.read_u32_le().await? as usize;
                    ensure!(len <= LIMIT, "Oversized activation request");
                    let mut bytes = vec![0; len];
                    client.read_exact(&mut bytes).await?;
                    let launch: Launch = serde_json::from_slice(&bytes)?;
                    ensure!(
                        launch.folder.as_ref().is_none_or(|p| p.is_absolute()),
                        "Activation folder must be absolute"
                    );
                    accept(launch)?;
                    client.write_u8(1).await?;
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn v94_relaunch_forwards_and_quit_releases_profile() {
        let name = format!(r"\\.\pipe\fastrock-test-{}", uuid::Uuid::new_v4());
        let launch = Launch {
            folder: Some(std::env::temp_dir()),
            resume: Some("goose:old-thread".into()),
        };
        let mut owner = acquire_at(name.clone(), launch.clone())
            .await
            .unwrap()
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        owner.listen(&tokio::runtime::Handle::current(), move |launch| {
            tx.send(launch)?;
            Ok(())
        });
        for _ in 0..3 {
            assert!(
                acquire_at(name.clone(), launch.clone())
                    .await
                    .unwrap()
                    .is_none()
            );
            let received = rx.recv().await.unwrap();
            assert_eq!(received.folder, launch.folder);
            assert_eq!(received.resume, launch.resume);
        }
        let task = owner.task.take().unwrap();
        task.abort();
        let _ = task.await;
        drop(owner);
        assert!(acquire_at(name, launch).await.unwrap().is_some());
    }
}
