//! A slow file association or network-link lookup must not block GPUI input.
use explorer_core::{CancelToken, platform};
use std::{path::PathBuf, sync::OnceLock};

pub enum OpenOutcome {
    Opened,
    Directory(PathBuf),
}
struct Request {
    path: PathBuf,
    resolve_link: bool,
    cancel: CancelToken,
    reply: async_channel::Sender<Result<OpenOutcome, String>>,
}

pub fn request(
    path: PathBuf,
    resolve_link: bool,
    cancel: CancelToken,
) -> Result<async_channel::Receiver<Result<OpenOutcome, String>>, String> {
    static WORKER: OnceLock<Result<std::sync::mpsc::SyncSender<Request>, String>> = OnceLock::new();
    let worker = WORKER
        .get_or_init(|| {
            let (sender, receiver) = std::sync::mpsc::sync_channel::<Request>(4);
            std::thread::Builder::new()
                .name("open-files".into())
                .spawn(move || {
                    #[cfg(windows)]
                    let apartment = unsafe { windows::Win32::System::Ole::OleInitialize(None) };
                    while let Ok(request) = receiver.recv() {
                        if request.cancel.is_cancelled() || request.reply.is_closed() {
                            continue;
                        }
                        #[cfg(windows)]
                        if let Err(error) = &apartment {
                            let _ = request.reply.try_send(Err(format!(
                                "ファイルを開く準備に失敗しました: {error}"
                            )));
                            continue;
                        }
                        let result = if request.resolve_link && request.path.is_dir() {
                            Ok(OpenOutcome::Directory(request.path))
                        } else if request.cancel.is_cancelled() || request.reply.is_closed() {
                            continue;
                        } else {
                            let opened = if crate::media::is_image_path(&request.path) {
                                platform::open_image_with_default_app(&request.path)
                            } else {
                                platform::open_with_default_app(&request.path)
                            };
                            opened
                                .map(|_| OpenOutcome::Opened)
                                .map_err(|e| e.to_string())
                        };
                        let _ = request.reply.try_send(result);
                    }
                    #[cfg(windows)]
                    if apartment.is_ok() {
                        unsafe {
                            windows::Win32::System::Ole::OleUninitialize();
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(sender)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (reply, receiver) = async_channel::bounded(1);
    worker
        .try_send(Request {
            path,
            resolve_link,
            cancel,
            reply,
        })
        .map_err(|_| {
            "ファイルを開く処理が混み合っています。少し待って再試行してください".to_string()
        })?;
    Ok(receiver)
}
