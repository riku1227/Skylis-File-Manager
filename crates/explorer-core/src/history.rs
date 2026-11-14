use std::path::{Path, PathBuf};

/// 戻る/進む の履歴。移動元フォルダーで選択していた名前も覚えておき、戻ったときに選択を復元する。
#[derive(Debug, Clone)]
pub struct History {
    back: Vec<(PathBuf, Option<String>)>,
    forward: Vec<(PathBuf, Option<String>)>,
    current: PathBuf,
}

impl History {
    pub fn new(start: PathBuf) -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
            current: start,
        }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    /// 新しい場所へ移動。`selected` は今の場所で選んでいた項目名。
    pub fn push(&mut self, to: PathBuf, selected: Option<String>) {
        if to == self.current {
            return;
        }
        let from = std::mem::replace(&mut self.current, to);
        self.back.push((from, selected));
        self.forward.clear();
    }

    /// 戻る。戻り先と、そこで選んでいた項目名を返す。
    pub fn go_back(&mut self, selected: Option<String>) -> Option<(PathBuf, Option<String>)> {
        let (to, sel) = self.back.pop()?;
        let from = std::mem::replace(&mut self.current, to.clone());
        self.forward.push((from, selected));
        Some((to, sel))
    }

    pub fn go_forward(&mut self, selected: Option<String>) -> Option<(PathBuf, Option<String>)> {
        let (to, sel) = self.forward.pop()?;
        let from = std::mem::replace(&mut self.current, to.clone());
        self.back.push((from, selected));
        Some((to, sel))
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

/// パンくず用に、パスを (表示名, そこまでのパス) に分解する
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    for comp in path.components() {
        acc.push(comp.as_os_str());
        let label = match comp {
            std::path::Component::Prefix(p) => p.as_os_str().to_string_lossy().into_owned(),
            std::path::Component::RootDir => {
                // Windowsでは "C:" + "\" をまとめて1つにする
                if out.is_empty() {
                    "/".into()
                } else {
                    continue;
                }
            }
            other => other.as_os_str().to_string_lossy().into_owned(),
        };
        out.push((label, acc.clone()));
    }
    // Windowsのドライブ部分は "C:\" を指すように補正
    if let Some(first) = out.first_mut() {
        if first.1.as_os_str().len() == 2 && first.0.ends_with(':') {
            first.1.push("\\");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_forward_restores_selection() {
        let mut h = History::new("a".into());
        h.push("b".into(), Some("b".into()));
        h.push("c".into(), Some("x".into()));
        assert_eq!(
            h.go_back(Some("y".into())),
            Some(("b".into(), Some("x".into())))
        );
        assert_eq!(h.go_forward(None), Some(("c".into(), Some("y".into()))));
        assert!(!h.can_go_forward());
    }

    #[cfg(windows)]
    #[test]
    fn breadcrumbs_windows() {
        let b = breadcrumbs(Path::new(r"C:\Users\me"));
        let labels: Vec<_> = b.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(labels, vec!["C:", "Users", "me"]);
        assert_eq!(b[0].1, PathBuf::from(r"C:\"));
        assert_eq!(b[2].1, PathBuf::from(r"C:\Users\me"));
    }
}
