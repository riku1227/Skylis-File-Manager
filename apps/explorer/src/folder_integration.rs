//! Optional per-user folder association. Registry access runs only on a worker.
//! Each changed value has a durable journal; unrelated later changes are preserved.
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Value {
    kind: u32,
    data: Vec<u8>,
}
impl Value {
    fn string(text: &str) -> Self {
        Self {
            kind: 1,
            data: text
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        }
    }
}
#[derive(Clone)]
struct Change {
    key: String,
    name: &'static str,
    value: Value,
}
fn plan(exe: &Path) -> Result<Vec<Change>, String> {
    let exe = exe.to_str().ok_or("実行ファイルのパスを登録できません")?;
    if !Path::new(exe).is_absolute() || exe.len() > 8192 || exe.contains(['"', '\0', '\r', '\n']) {
        return Err("実行ファイルのパスを登録できません".into());
    }
    // End with a dot component so a drive root's trailing slash cannot escape the quote.
    let command = format!("\"{exe}\" --shell-folder \"%1\\.\"");
    let mut changes = Vec::new();
    for class in ["Directory", "Drive"] {
        for (suffix, name, text) in [
            ("\\Skylis.Open", "", "Skylisで開く"),
            ("\\Skylis.Open\\command", "", command.as_str()),
            ("\\open\\command", "DelegateExecute", ""),
            ("\\open\\command", "", command.as_str()),
            ("", "", "Skylis.Open"),
        ] {
            changes.push(Change {
                key: format!("Software\\Classes\\{class}\\shell{suffix}"),
                name,
                value: Value::string(text),
            });
        }
    }
    Ok(changes)
}

trait Registry {
    fn read(&self, key: &str, name: &str) -> Result<Option<Value>, String>;
    fn write(&self, key: &str, name: &str, value: Option<&Value>) -> Result<(), String>;
    fn key_exists(&self, key: &str) -> Result<bool, String>;
    /// Returns true only when this call creates the key.
    fn create_key(&self, key: &str) -> Result<bool, String>;
    /// Never recursively deletes or removes a key containing any values/subkeys.
    fn delete_empty_key(&self, key: &str) -> Result<bool, String>;
}
const JOURNAL: &str = "Software\\Skylis\\FolderIntegrationBackupV1";
const WIN_E_JOURNAL: &str = "Software\\Skylis\\WinEIntegrationBackupV1";
const SWITCH_JOURNAL: &str = "Software\\Skylis\\SwitchIntegrationBackupV1";
pub const SWITCH_CLSID: &str = "{8459D65F-75C4-4AA0-B873-147A17E12C60}";

#[derive(Clone, Copy, Debug)]
pub enum Integration {
    Folders,
    WinE,
    Switch,
}

// V1's value and key indices are persisted. Keep plan() and its key order frozen.
// New registrations have their own journal, including when upgrading an enabled V1.
fn capability_plan(kind: Integration, exe: &Path) -> Result<Vec<Change>, String> {
    let base = plan(exe)?;
    if matches!(kind, Integration::Folders) {
        return Ok(base);
    }
    let exe = exe.to_str().ok_or("実行ファイルのパスを登録できません")?;
    let win_e = "Software\\Classes\\CLSID\\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\\shell\\opennewwindow\\command";
    let background = "Software\\Classes\\Directory\\Background\\shell\\Skylis.Switch";
    if matches!(kind, Integration::WinE) {
        return Ok(vec![
            Change {
                key: win_e.into(),
                name: "",
                value: Value::string(&format!("\"{exe}\" --home")),
            },
            Change {
                key: win_e.into(),
                name: "DelegateExecute",
                value: Value::string(""),
            },
        ]);
    }
    Ok(vec![
        Change {
            key: format!("Software\\Classes\\CLSID\\{SWITCH_CLSID}\\LocalServer32"),
            name: "",
            value: Value::string(&format!("\"{exe}\" --shell-command-server")),
        },
        Change {
            key: background.into(),
            name: "",
            value: Value::string("Skylisへ切り替え"),
        },
        Change {
            key: background.into(),
            name: "Icon",
            value: Value::string(&format!("\"{exe}\",0")),
        },
        Change {
            key: format!("{background}\\command"),
            name: "",
            value: Value::string(&format!("\"{exe}\" --switch-explorer \"%V\\.\"")),
        },
        Change {
            key: format!("{background}\\command"),
            name: "DelegateExecute",
            value: Value::string(SWITCH_CLSID),
        },
    ])
}

fn journal(kind: Integration) -> &'static str {
    match kind {
        Integration::Folders => JOURNAL,
        Integration::WinE => WIN_E_JOURNAL,
        Integration::Switch => SWITCH_JOURNAL,
    }
}
struct ScopedRegistry<'a, R> {
    registry: &'a R,
    journal: &'static str,
}
impl<R: Registry> Registry for ScopedRegistry<'_, R> {
    fn read(&self, key: &str, name: &str) -> Result<Option<Value>, String> {
        self.registry
            .read(if key == JOURNAL { self.journal } else { key }, name)
    }
    fn write(&self, key: &str, name: &str, value: Option<&Value>) -> Result<(), String> {
        self.registry
            .write(if key == JOURNAL { self.journal } else { key }, name, value)
    }
    fn key_exists(&self, key: &str) -> Result<bool, String> {
        self.registry.key_exists(key)
    }
    fn create_key(&self, key: &str) -> Result<bool, String> {
        self.registry.create_key(key)
    }
    fn delete_empty_key(&self, key: &str) -> Result<bool, String> {
        self.registry.delete_empty_key(key)
    }
}
const KEY_PREEXISTING: u8 = 0;
const KEY_PENDING: u8 = 1;
const KEY_CREATED: u8 = 2;

fn key_paths(changes: &[Change]) -> Vec<String> {
    let mut keys = Vec::new();
    for change in changes {
        let mut path = String::new();
        for component in change.key.split('\\') {
            if !path.is_empty() {
                path.push('\\');
            }
            path.push_str(component);
            if !keys.contains(&path) {
                keys.push(path.clone());
            }
        }
    }
    keys
}
fn key_record(state: u8) -> Value {
    Value {
        kind: 3,
        data: vec![state],
    }
}
fn key_state(value: &Value) -> Result<u8, String> {
    match value.data.as_slice() {
        [state @ (KEY_PREEXISTING | KEY_PENDING | KEY_CREATED)] if value.kind == 3 => Ok(*state),
        _ => Err("フォルダー連携のキー復元情報が壊れています".into()),
    }
}
fn prepare_keys(registry: &impl Registry, changes: &[Change]) -> Result<(), String> {
    for (i, key) in key_paths(changes).iter().enumerate() {
        let name = format!("key:{i}");
        if let Some(record) = registry.read(JOURNAL, &name)? {
            let state = key_state(&record)?;
            if state == KEY_PENDING && registry.key_exists(key)? {
                return Err("中断されたキー作成を確認できません。復元情報を保持しています".into());
            }
            registry.create_key(key)?;
            if state == KEY_PENDING {
                registry.write(JOURNAL, &name, Some(&key_record(KEY_CREATED)))?;
            }
        } else if registry.key_exists(key)? {
            registry.write(JOURNAL, &name, Some(&key_record(KEY_PREEXISTING)))?;
        } else {
            // Persist intent first. An interrupted unconfirmed create is never assumed owned.
            registry.write(JOURNAL, &name, Some(&key_record(KEY_PENDING)))?;
            let created = registry.create_key(key)?;
            registry.write(
                JOURNAL,
                &name,
                Some(&key_record(if created {
                    KEY_CREATED
                } else {
                    KEY_PREEXISTING
                })),
            )?;
        }
    }
    Ok(())
}
fn restore_keys(registry: &impl Registry, changes: &[Change]) -> Result<bool, String> {
    let mut complete = true;
    for (i, key) in key_paths(changes).iter().enumerate().rev() {
        let name = format!("key:{i}");
        let Some(record) = registry.read(JOURNAL, &name)? else {
            continue;
        };
        let done = match key_state(&record)? {
            KEY_PREEXISTING => true,
            KEY_CREATED => registry.delete_empty_key(key)?,
            KEY_PENDING => !registry.key_exists(key)?,
            _ => unreachable!(),
        };
        if done {
            registry.write(JOURNAL, &name, None)?;
        } else {
            complete = false;
        }
    }
    Ok(complete)
}

fn pack(previous: Option<&Value>, installed: &Value) -> Value {
    let mut data = Vec::new();
    for value in [previous, Some(installed)] {
        data.extend_from_slice(&value.map_or(u32::MAX, |v| v.kind).to_le_bytes());
        data.extend_from_slice(&(value.map_or(0, |v| v.data.len()) as u32).to_le_bytes());
        if let Some(value) = value {
            data.extend_from_slice(&value.data);
        }
    }
    Value { kind: 3, data }
}
fn unpack(value: &Value) -> Result<(Option<Value>, Value), String> {
    let mut bytes = value.data.as_slice();
    let mut next = || -> Option<Option<Value>> {
        if bytes.len() < 8 {
            return None;
        }
        let kind = u32::from_le_bytes(bytes[..4].try_into().ok()?);
        let len = u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize;
        bytes = bytes.get(8..)?;
        let data = bytes.get(..len)?.to_vec();
        bytes = bytes.get(len..)?;
        Some((kind != u32::MAX).then_some(Value { kind, data }))
    };
    let old = next();
    let installed = next();
    match (old, installed) {
        (Some(old), Some(Some(installed))) if bytes.is_empty() && value.kind == 3 => {
            Ok((old, installed))
        }
        _ => Err("フォルダー連携の復元情報が壊れています".into()),
    }
}

fn restore(registry: &impl Registry, changes: &[Change]) -> Result<(), String> {
    let mut conflict = false;
    for (i, change) in changes.iter().enumerate().rev() {
        let name = i.to_string();
        let Some(backup) = registry.read(JOURNAL, &name)? else {
            continue;
        };
        let (previous, installed) = unpack(&backup)?;
        let current = registry.read(&change.key, change.name)?;
        if current.as_ref() == Some(&installed) {
            registry.write(&change.key, change.name, previous.as_ref())?;
        } else if current != previous {
            conflict = true;
            continue;
        }
        registry.write(JOURNAL, &name, None)?;
    }
    conflict |= !restore_keys(registry, changes)?;
    if conflict {
        Err("別のアプリが変更した関連付けは保持しました。復元情報も残しています".into())
    } else {
        Ok(())
    }
}

fn install(registry: &impl Registry, changes: &[Change]) -> Result<(), String> {
    // Preflight every value before changing anything, including unfinished prior runs.
    for (i, change) in changes.iter().enumerate() {
        if registry
            .read(&change.key, change.name)?
            .is_some_and(|v| v.data.len() > 16_384)
        {
            return Err("既存の関連付けの値が大きすぎます".into());
        }
        if let Some(backup) = registry.read(JOURNAL, &i.to_string())? {
            let (previous, installed) = unpack(&backup)?;
            if installed != change.value {
                return Err("Skylisの場所が変わっています。先に元の設定に戻してください".into());
            }
            let current = registry.read(&change.key, change.name)?;
            if current.as_ref() != Some(&installed) && current != previous {
                return Err("別のアプリが関連付けを変更しています。先に復元してください".into());
            }
        }
    }
    let result = (|| {
        prepare_keys(registry, changes)?;
        for (i, change) in changes.iter().enumerate() {
            let name = i.to_string();
            let previous = if let Some(backup) = registry.read(JOURNAL, &name)? {
                unpack(&backup)?.0
            } else {
                registry.read(&change.key, change.name)?
            };
            registry.write(
                JOURNAL,
                &name,
                Some(&pack(previous.as_ref(), &change.value)),
            )?;
            registry.write(&change.key, change.name, Some(&change.value))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = restore(registry, changes);
    }
    result
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityStatus {
    pub enabled: bool,
    pub can_restore: bool,
}

fn plan_status(registry: &impl Registry, changes: &[Change]) -> Result<CapabilityStatus, String> {
    let mut status = CapabilityStatus {
        enabled: true,
        can_restore: false,
    };
    for (i, change) in changes.iter().enumerate() {
        status.enabled &= registry.read(&change.key, change.name)?.as_ref() == Some(&change.value);
        status.can_restore |= registry.read(JOURNAL, &i.to_string())?.is_some();
    }
    for i in 0..key_paths(changes).len() {
        status.can_restore |= registry.read(JOURNAL, &format!("key:{i}"))?.is_some();
    }
    Ok(status)
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub folders: CapabilityStatus,
    pub win_e: CapabilityStatus,
    pub switch: CapabilityStatus,
}
impl Status {
    pub fn get(&self, kind: Integration) -> &CapabilityStatus {
        match kind {
            Integration::Folders => &self.folders,
            Integration::WinE => &self.win_e,
            Integration::Switch => &self.switch,
        }
    }
}
pub fn query() -> Result<Status, String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let registry = windows::Registry;
        let status = |kind| {
            plan_status(
                &ScopedRegistry {
                    registry: &registry,
                    journal: journal(kind),
                },
                &capability_plan(kind, &exe)?,
            )
        };
        Ok(Status {
            folders: status(Integration::Folders)?,
            win_e: status(Integration::WinE)?,
            switch: status(Integration::Switch)?,
        })
    }
    #[cfg(not(windows))]
    {
        Err("この連携はWindowsのみ対応しています".into())
    }
}
pub fn set_enabled(kind: Integration, enabled: bool) -> Result<Status, String> {
    #[cfg(windows)]
    {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().map_err(|_| "フォルダー連携が使用中です")?;
        let _process_guard = windows::lock()?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let changes = capability_plan(kind, &exe)?;
        let registry = ScopedRegistry {
            registry: &windows::Registry,
            journal: journal(kind),
        };
        let result = if enabled {
            install(&registry, &changes)
        } else {
            restore(&registry, &changes)
        };
        // SAFETY: no pointer parameters are used for an association-change notification.
        unsafe {
            windows_sys::Win32::UI::Shell::SHChangeNotify(
                0x08000000,
                0,
                std::ptr::null(),
                std::ptr::null(),
            );
        }
        result?;
        query()
    }
    #[cfg(not(windows))]
    {
        let _ = (kind, enabled);
        Err("この連携はWindowsのみ対応しています".into())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::System::Registry::*;
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    fn error(code: u32) -> String {
        format!(
            "Windowsのフォルダー連携: {}",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }
    pub(super) struct Registry;
    pub(super) struct ProcessLock(windows_sys::Win32::Foundation::HANDLE);
    impl Drop for ProcessLock {
        fn drop(&mut self) {
            unsafe {
                windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
    pub(super) fn lock() -> Result<ProcessLock, String> {
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        let handle = unsafe {
            CreateMutexW(
                std::ptr::null(),
                0,
                wide("Local\\Skylis.FolderIntegration").as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let result = unsafe { WaitForSingleObject(handle, 5000) };
        if result == 0 || result == 0x80 {
            Ok(ProcessLock(handle))
        } else {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
            Err("別のSkylisがフォルダー連携を変更中です".into())
        }
    }
    impl super::Registry for Registry {
        fn key_exists(&self, path: &str) -> Result<bool, String> {
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    KEY_READ,
                    &mut handle,
                )
            };
            if result == 2 {
                return Ok(false);
            }
            if result != 0 {
                return Err(error(result));
            }
            drop(Key(handle));
            Ok(true)
        }
        fn create_key(&self, path: &str) -> Result<bool, String> {
            let mut handle = std::ptr::null_mut();
            let mut disposition = 0;
            let result = unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    std::ptr::null(),
                    0,
                    KEY_SET_VALUE | KEY_QUERY_VALUE,
                    std::ptr::null(),
                    &mut handle,
                    &mut disposition,
                )
            };
            if result != 0 {
                return Err(error(result));
            }
            drop(Key(handle));
            Ok(disposition == REG_CREATED_NEW_KEY)
        }
        fn delete_empty_key(&self, path: &str) -> Result<bool, String> {
            use windows_sys::Win32::{
                Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
                Storage::FileSystem::{CommitTransaction, CreateTransaction},
            };
            struct Transaction(HANDLE);
            impl Drop for Transaction {
                fn drop(&mut self) {
                    unsafe {
                        CloseHandle(self.0);
                    }
                }
            }
            // A non-transacted write between the emptiness check and deletion aborts
            // this transaction instead of deleting another application's new value.
            let transaction = unsafe {
                CreateTransaction(
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    5000,
                    std::ptr::null(),
                )
            };
            if transaction == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let transaction = Transaction(transaction);
            let mut handle = std::ptr::null_mut();
            let path = wide(path);
            let result = unsafe {
                RegOpenKeyTransactedW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    KEY_READ,
                    &mut handle,
                    transaction.0,
                    std::ptr::null(),
                )
            };
            if result == 2 {
                return Ok(true);
            }
            if result != 0 {
                return Err(error(result));
            }
            let key = Key(handle);
            let mut subkeys = 0;
            let mut values = 0;
            let result = unsafe {
                RegQueryInfoKeyW(
                    key.0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &mut subkeys,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut values,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if result != 0 {
                return Err(error(result));
            }
            if subkeys != 0 || values != 0 {
                return Ok(false);
            }
            let result = unsafe {
                RegDeleteKeyTransactedW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    0,
                    transaction.0,
                    std::ptr::null(),
                )
            };
            if result != 0 {
                return Err(error(result));
            }
            drop(key);
            if unsafe { CommitTransaction(transaction.0) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(true)
        }
        fn read(&self, path: &str, name: &str) -> Result<Option<Value>, String> {
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    wide(path).as_ptr(),
                    0,
                    KEY_READ,
                    &mut handle,
                )
            };
            if result == 2 {
                return Ok(None);
            }
            if result != 0 {
                return Err(error(result));
            }
            let key = Key(handle);
            let name = wide(name);
            let mut kind = 0;
            let mut len = 0;
            let result = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    std::ptr::null_mut(),
                    &mut len,
                )
            };
            if result == 2 {
                return Ok(None);
            }
            if result != 0 {
                return Err(error(result));
            }
            if len > 65_536 {
                return Err("関連付けの値が大きすぎます".into());
            }
            let mut data = vec![0; len as usize];
            let result = unsafe {
                RegQueryValueExW(
                    key.0,
                    name.as_ptr(),
                    std::ptr::null(),
                    &mut kind,
                    data.as_mut_ptr(),
                    &mut len,
                )
            };
            if result != 0 {
                return Err(error(result));
            }
            data.truncate(len as usize);
            Ok(Some(Value { kind, data }))
        }
        fn write(&self, path: &str, name: &str, value: Option<&Value>) -> Result<(), String> {
            let mut handle = std::ptr::null_mut();
            let result = unsafe {
                if value.is_none() {
                    RegOpenKeyExW(
                        HKEY_CURRENT_USER,
                        wide(path).as_ptr(),
                        0,
                        KEY_SET_VALUE | KEY_QUERY_VALUE,
                        &mut handle,
                    )
                } else {
                    RegCreateKeyExW(
                        HKEY_CURRENT_USER,
                        wide(path).as_ptr(),
                        0,
                        std::ptr::null(),
                        0,
                        KEY_SET_VALUE | KEY_QUERY_VALUE,
                        std::ptr::null(),
                        &mut handle,
                        std::ptr::null_mut(),
                    )
                }
            };
            if result == 2 && value.is_none() {
                return Ok(());
            }
            if result != 0 {
                return Err(error(result));
            }
            let key = Key(handle);
            let result = unsafe {
                if let Some(value) = value {
                    RegSetValueExW(
                        key.0,
                        wide(name).as_ptr(),
                        0,
                        value.kind,
                        value.data.as_ptr(),
                        value.data.len() as u32,
                    )
                } else {
                    RegDeleteValueW(key.0, wide(name).as_ptr())
                }
            };
            if result != 0 && !(value.is_none() && result == 2) {
                return Err(error(result));
            }
            let result = unsafe { RegFlushKey(key.0) };
            if result != 0 {
                return Err(error(result));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        collections::{BTreeSet, HashMap},
    };
    struct Fake(
        RefCell<HashMap<(String, String), Value>>,
        RefCell<BTreeSet<String>>,
    );
    impl Default for Fake {
        fn default() -> Self {
            Self(
                RefCell::new(HashMap::new()),
                RefCell::new(BTreeSet::from([
                    "Software".into(),
                    "Software\\Classes".into(),
                ])),
            )
        }
    }
    impl Registry for Fake {
        fn key_exists(&self, key: &str) -> Result<bool, String> {
            Ok(self.1.borrow().contains(key))
        }
        fn create_key(&self, key: &str) -> Result<bool, String> {
            let existed = self.key_exists(key)?;
            let mut path = String::new();
            for component in key.split('\\') {
                if !path.is_empty() {
                    path.push('\\');
                }
                path.push_str(component);
                self.1.borrow_mut().insert(path.clone());
            }
            Ok(!existed)
        }
        fn delete_empty_key(&self, key: &str) -> Result<bool, String> {
            if self.0.borrow().keys().any(|(k, _)| k == key)
                || self
                    .1
                    .borrow()
                    .iter()
                    .any(|k| k.starts_with(&format!("{key}\\")))
            {
                return Ok(false);
            }
            self.1.borrow_mut().remove(key);
            Ok(true)
        }
        fn read(&self, key: &str, name: &str) -> Result<Option<Value>, String> {
            Ok(self.0.borrow().get(&(key.into(), name.into())).cloned())
        }
        fn write(&self, key: &str, name: &str, value: Option<&Value>) -> Result<(), String> {
            if let Some(value) = value {
                self.create_key(key)?;
                self.0
                    .borrow_mut()
                    .insert((key.into(), name.into()), value.clone());
            } else {
                self.0.borrow_mut().remove(&(key.into(), name.into()));
            }
            Ok(())
        }
    }
    fn changes() -> Vec<Change> {
        plan(if cfg!(windows) {
            Path::new("C:\\Program Files\\Skylis\\skylis.exe")
        } else {
            Path::new("/opt/Skylis/explorer")
        })
        .unwrap()
    }
    #[test]
    fn capabilities_have_independent_journals_and_do_not_reindex_v1() {
        let registry = Fake::default();
        let exe = if cfg!(windows) {
            Path::new("C:\\Program Files\\Skylis\\skylis.exe")
        } else {
            Path::new("/opt/Skylis/explorer")
        };
        let base = plan(exe).unwrap();
        assert_eq!(base.len(), 10);
        assert_eq!(
            base[0].key,
            "Software\\Classes\\Directory\\shell\\Skylis.Open"
        );
        assert_eq!(base[4].key, "Software\\Classes\\Directory\\shell");
        assert_eq!(base[5].key, "Software\\Classes\\Drive\\shell\\Skylis.Open");
        assert_eq!(base[9].key, "Software\\Classes\\Drive\\shell");
        install(&registry, &base).unwrap();
        let before: HashMap<_, _> = registry
            .0
            .borrow()
            .iter()
            .filter(|((key, _), _)| key == JOURNAL)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for kind in [Integration::WinE, Integration::Switch] {
            let changes = capability_plan(kind, exe).unwrap();
            assert!(
                changes
                    .iter()
                    .all(|c| !c.key.starts_with("Software\\Classes\\Folder\\"))
            );
            let scoped = ScopedRegistry {
                registry: &registry,
                journal: journal(kind),
            };
            install(&scoped, &changes).unwrap();
            assert!(plan_status(&scoped, &changes).unwrap().enabled);
            restore(&scoped, &changes).unwrap();
            assert!(!plan_status(&scoped, &changes).unwrap().enabled);
            assert!(!plan_status(&scoped, &changes).unwrap().can_restore);
            let after: HashMap<_, _> = registry
                .0
                .borrow()
                .iter()
                .filter(|((key, _), _)| key == JOURNAL)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            assert_eq!(before, after);
            assert!(plan_status(&registry, &base).unwrap().enabled);
        }
        restore(&registry, &base).unwrap();
        assert!(registry.0.borrow().is_empty());
    }
    #[test]
    fn extension_restoration_preserves_foreign_commands_and_their_journal() {
        let registry = Fake::default();
        let exe = if cfg!(windows) {
            Path::new("C:\\Skylis\\skylis.exe")
        } else {
            Path::new("/opt/skylis")
        };
        let changes = capability_plan(Integration::WinE, exe).unwrap();
        let scoped = ScopedRegistry {
            registry: &registry,
            journal: WIN_E_JOURNAL,
        };
        install(&scoped, &changes).unwrap();
        let foreign = Value::string("foreign.exe");
        registry.write(&changes[0].key, "", Some(&foreign)).unwrap();
        assert!(restore(&scoped, &changes).is_err());
        assert_eq!(registry.read(&changes[0].key, "").unwrap(), Some(foreign));
        assert!(registry.read(WIN_E_JOURNAL, "0").unwrap().is_some());
        assert!(registry.read(JOURNAL, "0").unwrap().is_none());
    }
    #[test]
    fn registration_restores_exact_prior_types_and_absence() {
        let registry = Fake::default();
        let changes = changes();
        let old = Value {
            kind: 2,
            data: vec![1, 0, 2, 0],
        };
        registry
            .write(&changes[0].key, changes[0].name, Some(&old))
            .unwrap();
        install(&registry, &changes).unwrap();
        install(&registry, &changes).unwrap();
        restore(&registry, &changes).unwrap();
        assert_eq!(
            registry.read(&changes[0].key, changes[0].name).unwrap(),
            Some(old)
        );
        assert_eq!(registry.0.borrow().len(), 1);
    }
    #[test]
    fn restore_keeps_third_party_changes_and_their_backup() {
        let registry = Fake::default();
        let changes = changes();
        install(&registry, &changes).unwrap();
        let other = Value::string("AnotherManager");
        registry
            .write(&changes[0].key, changes[0].name, Some(&other))
            .unwrap();
        assert!(restore(&registry, &changes).is_err());
        assert_eq!(
            registry.read(&changes[0].key, changes[0].name).unwrap(),
            Some(other)
        );
        assert!(registry.read(JOURNAL, "0").unwrap().is_some());
        assert!(registry.key_exists(&changes[0].key).unwrap());
    }
    #[test]
    fn recovery_before_target_write_removes_only_journal() {
        let registry = Fake::default();
        let changes = changes();
        registry
            .write(JOURNAL, "0", Some(&pack(None, &changes[0].value)))
            .unwrap();
        restore(&registry, &changes).unwrap();
        assert!(registry.0.borrow().is_empty());
    }
    #[test]
    fn oversized_existing_value_is_rejected_before_any_write() {
        let registry = Fake::default();
        let changes = changes();
        registry
            .write(
                &changes[8].key,
                changes[8].name,
                Some(&Value {
                    kind: 3,
                    data: vec![0; 16_385],
                }),
            )
            .unwrap();
        let before = registry.0.borrow().clone();
        assert!(install(&registry, &changes).is_err());
        assert_eq!(*registry.0.borrow(), before);
    }
    #[test]
    fn interrupted_install_rolls_back_earlier_values() {
        struct FailOnce {
            inner: Fake,
            key: String,
            failed: std::cell::Cell<bool>,
        }
        impl Registry for FailOnce {
            fn key_exists(&self, key: &str) -> Result<bool, String> {
                self.inner.key_exists(key)
            }
            fn create_key(&self, key: &str) -> Result<bool, String> {
                self.inner.create_key(key)
            }
            fn delete_empty_key(&self, key: &str) -> Result<bool, String> {
                self.inner.delete_empty_key(key)
            }
            fn read(&self, key: &str, name: &str) -> Result<Option<Value>, String> {
                self.inner.read(key, name)
            }
            fn write(&self, key: &str, name: &str, value: Option<&Value>) -> Result<(), String> {
                if key == self.key && !self.failed.replace(true) {
                    return Err("injected failure".into());
                }
                self.inner.write(key, name, value)
            }
        }
        let changes = changes();
        let registry = FailOnce {
            inner: Fake::default(),
            key: changes[3].key.clone(),
            failed: std::cell::Cell::new(false),
        };
        assert!(install(&registry, &changes).is_err());
        assert!(registry.inner.0.borrow().is_empty());
        assert!(
            key_paths(&changes)
                .iter()
                .filter(|k| k.starts_with("Software\\Classes\\"))
                .all(|k| !registry.inner.key_exists(k).unwrap())
        );
    }
    #[test]
    fn restoration_removes_created_empty_keys_and_preserves_preexisting_empty_keys() {
        let registry = Fake::default();
        let changes = changes();
        // An intentionally empty open-command key belongs to another installation.
        registry.create_key(&changes[2].key).unwrap();
        let before = registry.1.borrow().clone();
        install(&registry, &changes).unwrap();
        restore(&registry, &changes).unwrap();
        let actual: BTreeSet<_> = registry
            .1
            .borrow()
            .iter()
            .filter(|k| k.starts_with("Software\\Classes"))
            .cloned()
            .collect();
        let expected: BTreeSet<_> = before
            .into_iter()
            .filter(|k| k.starts_with("Software\\Classes"))
            .collect();
        assert_eq!(actual, expected);
        assert!(registry.0.borrow().is_empty());
    }
    #[test]
    fn created_keys_with_foreign_values_or_children_are_never_removed() {
        let registry = Fake::default();
        let changes = changes();
        install(&registry, &changes).unwrap();
        registry
            .write(&changes[1].key, "Foreign", Some(&Value::string("preserve")))
            .unwrap();
        let child = format!("{}\\ForeignChild", changes[7].key);
        registry.create_key(&child).unwrap();
        assert!(restore(&registry, &changes).is_err());
        assert!(registry.read(&changes[1].key, "Foreign").unwrap().is_some());
        assert!(registry.key_exists(&child).unwrap());
        assert!(registry.key_exists(&changes[1].key).unwrap());
    }
    #[test]
    fn recovery_cleans_confirmed_keys_but_keeps_unconfirmed_existing_key() {
        let registry = Fake::default();
        let changes = changes();
        prepare_keys(&registry, &changes).unwrap();
        restore(&registry, &changes).unwrap();
        assert!(!registry.key_exists(&changes[0].key).unwrap());
        let keys = key_paths(&changes);
        let index = keys.iter().position(|k| k == &changes[0].key).unwrap();
        registry
            .write(
                JOURNAL,
                &format!("key:{index}"),
                Some(&key_record(KEY_PENDING)),
            )
            .unwrap();
        registry.create_key(&changes[0].key).unwrap();
        assert!(restore(&registry, &changes).is_err());
        assert!(registry.key_exists(&changes[0].key).unwrap());
        assert!(
            registry
                .read(JOURNAL, &format!("key:{index}"))
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn command_quotes_binary_and_protects_root_argument() {
        let changes = changes();
        let data = &changes[1].value.data;
        let command = String::from_utf16(
            &data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(command.starts_with('"'));
        assert!(command.ends_with("--shell-folder \"%1\\.\"\0"));
        assert_eq!(changes.len(), 10);
        assert!(plan(Path::new("relative.exe")).is_err());
        assert!(
            unpack(&Value {
                kind: 3,
                data: vec![0; 3]
            })
            .is_err()
        );
    }
}
