//! Observations about a user-requested listing, never a global TCC/FDA verdict.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Observation {
    #[default]
    Unknown,
    Denied,
    Accessible,
    OtherFailure,
}
impl Observation {
    pub fn record(&mut self, result: Result<(), std::io::ErrorKind>, obsolete: bool) {
        if obsolete {
            return;
        }
        *self = match result {
            Ok(()) => Self::Accessible,
            Err(std::io::ErrorKind::PermissionDenied) => Self::Denied,
            Err(_) => Self::OtherFailure,
        };
    }
}

pub fn bundle_for_executable(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observations_never_infer_full_disk_access() {
        let mut state = Observation::default();
        assert_eq!(state, Observation::Unknown);
        state.record(Err(std::io::ErrorKind::PermissionDenied), false);
        assert_eq!(state, Observation::Denied);
        state.record(Ok(()), true);
        assert_eq!(
            state,
            Observation::Denied,
            "stale success must not clear denial"
        );
        state.record(Ok(()), false);
        assert_eq!(state, Observation::Accessible);
        state.record(Err(std::io::ErrorKind::NotFound), false);
        assert_eq!(state, Observation::OtherFailure);
        state = Observation::default();
        assert_eq!(state, Observation::Unknown, "navigation resets evidence");
    }
    #[test]
    fn reveal_requires_a_real_bundle_layout_and_preserves_spaces() {
        assert_eq!(
            bundle_for_executable(Path::new("/Some Place/Skylis.app/Contents/MacOS/Skylis")),
            Some(PathBuf::from("/Some Place/Skylis.app"))
        );
        assert_eq!(bundle_for_executable(Path::new("/tmp/explorer")), None);
        assert_eq!(
            bundle_for_executable(Path::new("/tmp/fake.app/explorer")),
            None
        );
    }
}

#[derive(Clone, Copy)]
pub enum Action {
    OpenPrivacy,
    RevealBundle,
}

/// Called only on a bounded, user-initiated worker. `open` dispatches to Launch Services.
pub fn perform(action: Action) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/open");
        match action {
            Action::OpenPrivacy => {
                command.arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
                );
            }
            Action::RevealBundle => {
                let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                let bundle = bundle_for_executable(&executable)
                    .ok_or(".app として起動してください（scripts/build-macos.sh）")?;
                command.arg("-R").arg(bundle);
            }
        }
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "OS操作に失敗: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok("OSへ表示を依頼しました。表示されない場合は、この画面の手順で開いてください。".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = action;
        Err("macOSでのみ利用できます".into())
    }
}

/// Per-visit presentation state, independent of a global privacy authorization.
#[derive(Default)]
pub struct Notice {
    pub observation: Observation,
    dismissed: bool,
}
impl Notice {
    pub fn record(&mut self, result: Result<(), std::io::ErrorKind>, obsolete: bool) {
        if obsolete {
            return;
        }
        self.observation.record(result, false);
        if self.observation == Observation::Accessible {
            self.dismissed = false;
        }
    }
    pub fn visible(&self) -> bool {
        self.observation == Observation::Denied && !self.dismissed
    }
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }
    pub fn retry(&mut self) {
        self.dismissed = false;
    }
}

#[cfg(test)]
mod notice_tests {
    use super::*;
    #[test]
    fn dismissal_survives_automatic_denials_but_retry_and_success_reset_it() {
        let mut notice = Notice::default();
        notice.record(Err(std::io::ErrorKind::PermissionDenied), false);
        assert!(notice.visible());
        notice.dismiss();
        notice.record(Err(std::io::ErrorKind::PermissionDenied), false);
        assert!(!notice.visible());
        notice.record(Err(std::io::ErrorKind::NotFound), false);
        notice.record(Err(std::io::ErrorKind::PermissionDenied), false);
        assert!(
            !notice.visible(),
            "other automatic failures do not end dismissal"
        );
        notice.retry();
        assert!(notice.visible());
        notice.record(Ok(()), true);
        assert!(notice.visible());
        notice.record(Ok(()), false);
        assert!(!notice.visible());
        notice.record(Err(std::io::ErrorKind::NotFound), false);
        assert!(!notice.visible());
        notice.record(Err(std::io::ErrorKind::PermissionDenied), true);
        assert!(!notice.visible());
        notice.record(Err(std::io::ErrorKind::PermissionDenied), false);
        notice = Notice::default();
        assert!(!notice.visible());
    }
}
