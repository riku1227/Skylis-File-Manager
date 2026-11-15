//! Command-line decoding is filesystem-free; opening and selection stay with the listing worker.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeArgument {
    Light,
    Dark,
    System,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchRequest {
    pub path: Option<PathBuf>,
    pub selected_name: Option<String>,
    pub theme: Option<ThemeArgument>,
    pub switch_explorer: bool,
    pub handoff_token: Option<String>,
    pub shell_command_server: bool,
    pub error: Option<String>,
}

pub fn current() -> &'static LaunchRequest {
    static REQUEST: OnceLock<LaunchRequest> = OnceLock::new();
    REQUEST.get_or_init(|| {
        let cwd = std::env::current_dir().unwrap_or_else(|_| explorer_core::platform::home_dir());
        parse(std::env::args_os().skip(1), &cwd).unwrap_or_else(|error| LaunchRequest {
            error: Some(error),
            ..Default::default()
        })
    })
}

fn path_argument(value: OsString, cwd: &Path) -> Result<PathBuf, String> {
    if value.is_empty() || value.as_encoded_bytes().contains(&0) {
        return Err("起動時のパスが空か、使用できない文字を含んでいます".into());
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(cwd.join(path))
    }
}

pub fn valid_token(token: &str) -> bool {
    token.len() == 36
        && token.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

pub fn parse(
    args: impl IntoIterator<Item = OsString>,
    cwd: &Path,
) -> Result<LaunchRequest, String> {
    let mut request = LaunchRequest::default();
    let mut args = args.into_iter();
    let mut positional = false;
    let mut embedding = false;
    let mut target_seen = false;
    while let Some(arg) = args.next() {
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && arg == "--home" {
            if target_seen {
                return Err("起動時に指定できる場所は1つです".into());
            }
            target_seen = true;
            continue;
        }
        if !positional
            && arg.to_str().is_some_and(|s| {
                s.eq_ignore_ascii_case("-Embedding") || s.eq_ignore_ascii_case("/Embedding")
            })
        {
            embedding = true;
            continue;
        }
        if !positional && arg == "--shell-command-server" {
            request.shell_command_server = true;
            continue;
        }
        if !positional && arg == "--theme" {
            request.theme = Some(match args.next().as_deref().and_then(|v| v.to_str()) {
                Some("light") => ThemeArgument::Light,
                Some("dark") => ThemeArgument::Dark,
                Some("system") => ThemeArgument::System,
                _ => return Err("--theme は light / dark / system を指定してください".into()),
            });
            continue;
        }
        if !positional && arg == "--handoff-token" {
            let token = args
                .next()
                .and_then(|s| s.into_string().ok())
                .filter(|s| valid_token(s))
                .ok_or("切り替え要求の識別子が不正です")?;
            if request.handoff_token.replace(token).is_some() {
                return Err("切り替え要求が重複しています".into());
            }
            continue;
        }
        let select = !positional && arg == "--select";
        let switch = !positional && arg == "--switch-explorer";
        let value =
            if !positional && (arg == "--path" || arg == "--shell-folder" || select || switch) {
                args.next()
                    .ok_or_else(|| format!("{} の後にパスが必要です", arg.to_string_lossy()))?
            } else {
                if !positional && arg.to_string_lossy().starts_with('-') {
                    return Err(format!("不明な起動オプション: {}", arg.to_string_lossy()));
                }
                arg
            };
        if target_seen {
            return Err("起動時に指定できる場所は1つです".into());
        }
        target_seen = true;
        let path = path_argument(value, cwd)?;
        if select {
            let name = path
                .file_name()
                .ok_or("ドライブ全体は選択対象にできません")?;
            let parent = path
                .parent()
                .ok_or("選択対象の親フォルダーが見つかりません")?;
            request.selected_name = Some(name.to_string_lossy().into_owned());
            request.path = Some(parent.to_path_buf());
        } else {
            request.path = Some(path);
        }
        request.switch_explorer = switch;
    }
    if request.handoff_token.is_some() && (request.path.is_none() || request.shell_command_server) {
        return Err("切り替え要求にフォルダーがありません".into());
    }
    if request.shell_command_server && (target_seen || request.theme.is_some()) {
        return Err("Shell連携サーバーの起動引数が不正です".into());
    }
    if embedding && !request.shell_command_server {
        return Err("Shell連携サーバーの起動引数が不正です".into());
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cwd() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\作業")
        } else {
            PathBuf::from("/作業")
        }
    }
    fn request(args: &[&str]) -> Result<LaunchRequest, String> {
        parse(args.iter().map(OsString::from), &cwd())
    }

    #[test]
    fn unicode_selection_keeps_filename_and_parent() {
        let path = cwd().join("資料と画像").join("見積り 'final' & (2).txt");
        let r = parse(
            [OsString::from("--select"), path.clone().into_os_string()],
            &cwd(),
        )
        .unwrap();
        assert_eq!(r.path.as_deref(), path.parent());
        assert_eq!(r.selected_name.as_deref(), Some("見積り 'final' & (2).txt"));
    }
    #[test]
    fn relative_and_dash_paths_are_data() {
        assert!(request(&["--home"]).unwrap().path.is_none());
        assert!(request(&["--home", "--path", "folder"]).is_err());
        assert!(request(&["--path", "folder", "--home"]).is_err());
        assert_eq!(
            request(&["--", "-資料"]).unwrap().path,
            Some(cwd().join("-資料"))
        );
        assert_eq!(
            request(&["--path", "-資料"]).unwrap().path,
            Some(cwd().join("-資料"))
        );
        assert!(request(&["--path"]).is_err());
        assert!(request(&["--unknown"]).is_err());
        assert!(request(&["first", "second"]).is_err());
    }
    #[test]
    fn switch_is_explicit_and_handoff_tokens_are_bounded() {
        let r = request(&["--switch-explorer", "folder"]).unwrap();
        assert!(r.switch_explorer);
        assert!(r.handoff_token.is_none());
        assert!(!request(&["--path", "folder"]).unwrap().switch_explorer);
        assert!(valid_token("00112233-4455-6677-8899-aAbBcCdDeEfF"));
        assert!(!valid_token("..\\someone-elses-event"));
        assert!(request(&["--handoff-token", "00112233-4455-6677-8899-aabbccddeeff"]).is_err());
        assert!(
            request(&["--shell-command-server", "-Embedding"])
                .unwrap()
                .shell_command_server
        );
        assert!(request(&["-Embedding"]).is_err());
        assert!(request(&["--shell-command-server", "--path", "folder"]).is_err());
    }
    #[test]
    fn root_selection_and_empty_path_are_rejected() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(request(&["--select", root]).is_err());
        assert!(request(&["--path", ""]).is_err());
        assert!(request(&["--path", "a\0b"]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn unc_and_drive_roots_survive_registry_argument_protection() {
        let r = request(&["--shell-folder", r"\\server\share\."]).unwrap();
        assert_eq!(r.path, Some(PathBuf::from(r"\\server\share\.")));
        let r = request(&["--shell-folder", r"E:\."]).unwrap();
        assert_eq!(r.path, Some(PathBuf::from(r"E:\.")));
    }
}
