use crate::{
    batch,
    error::{Error, Result},
};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};
use windows::{
    core::w,
    Win32::{Foundation::*, System::Registry::*, UI::WindowsAndMessaging::*},
};

pub fn append_path(existing: &str, bin: &str) -> (String, bool) {
    fn normalized(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s.trim().trim_matches('"');
        while let Some(i) = rest.find('%') {
            out.push_str(&rest[..i]);
            rest = &rest[i + 1..];
            if let Some(end) = rest.find('%') {
                let name = &rest[..end];
                out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                rest = &rest[end + 1..];
            } else {
                out.push('%');
                break;
            }
        }
        out.push_str(rest);
        out.replace('/', "\\").trim_end_matches('\\').to_lowercase()
    }
    if existing
        .split(';')
        .any(|part| normalized(part) == normalized(bin))
    {
        return (existing.to_string(), false);
    }
    (
        if existing.is_empty() {
            bin.to_string()
        } else {
            format!("{existing};{bin}")
        },
        true,
    )
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
fn user_path(writable: bool) -> Result<(Key, String, REG_VALUE_TYPE)> {
    unsafe {
        let mut k = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Environment"),
            None,
            if writable {
                KEY_QUERY_VALUE | KEY_SET_VALUE
            } else {
                KEY_QUERY_VALUE
            },
            &mut k,
        )
        .ok()?;
        let key = Key(k);
        let mut length = 0;
        let mut ty = REG_EXPAND_SZ;
        let status = RegQueryValueExW(k, w!("Path"), None, Some(&mut ty), None, Some(&mut length));
        if status == ERROR_FILE_NOT_FOUND {
            return Ok((key, String::new(), REG_EXPAND_SZ));
        }
        status.ok()?;
        if ty != REG_SZ && ty != REG_EXPAND_SZ {
            return Err(Error::new(
                "install_failed",
                "User PATH is not a string registry value; it was left unchanged",
            ));
        }
        let mut data = vec![0u16; (length as usize).div_ceil(2) + 1];
        RegQueryValueExW(
            k,
            w!("Path"),
            None,
            Some(&mut ty),
            Some(data.as_mut_ptr().cast()),
            Some(&mut length),
        )
        .ok()?;
        let n = data.iter().position(|c| *c == 0).unwrap_or(data.len());
        Ok((
            key,
            String::from_utf16(&data[..n])
                .map_err(|_| Error::new("install_failed", "User PATH contains invalid UTF-16"))?,
            ty,
        ))
    }
}
pub fn copy_executable(source: &Path, target: &Path, replace: bool) -> Result<bool> {
    let cancel = Arc::new(AtomicBool::new(false));
    if target.exists() {
        if batch::hash_file(source, &cancel)? == batch::hash_file(target, &cancel)? {
            return Ok(false);
        }
        if !replace {
            return Err(Error::new(
                "output_exists",
                "A different acea.exe is installed; use install --replace true to replace it",
            ));
        }
    }
    std::fs::create_dir_all(target.parent().unwrap())?;
    let mut stage = tempfile::Builder::new()
        .prefix(".acea-install-")
        .suffix(".exe")
        .tempfile_in(target.parent().unwrap())?;
    let mut input = std::fs::File::open(source)?;
    std::io::copy(&mut input, &mut stage)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    if replace {
        stage.persist(target).map_err(|e|Error::new("install_failed",format!("Could not replace installed executable; close programs using it and retry. {e}")))?;
    } else {
        stage
            .persist_noclobber(target)
            .map_err(|e| Error::new("install_failed", e.to_string()))?;
    }
    Ok(true)
}
pub fn install(replace: bool, dry_run: bool) -> Result<Value> {
    let base = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::new("install_failed", "LOCALAPPDATA is unavailable"))?;
    let bin = PathBuf::from(base)
        .join("AIContentEditorAssistant")
        .join("bin");
    let target = bin.join("acea.exe");
    let (key, old, ty) = user_path(!dry_run)?;
    let (next, changed) = append_path(&old, &bin.to_string_lossy());
    if dry_run {
        return Ok(
            json!({"ok":true,"dry_run":true,"target":target,"path_change_required":changed,"user_path_type":ty.0,"note":"No files or PATH values changed"}),
        );
    }
    let copied = copy_executable(&std::env::current_exe()?, &target, replace)?;
    if changed {
        let utf16: Vec<u16> = next.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes =
            unsafe { std::slice::from_raw_parts(utf16.as_ptr().cast::<u8>(), utf16.len() * 2) };
        unsafe {
            RegSetValueExW(key.0, w!("Path"), None, ty, Some(bytes)).ok()?;
            let env = w!("Environment");
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(env.0 as isize),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
    }
    Ok(
        json!({"ok":true,"installed":target,"copied":copied,"user_path_updated":changed,"note":"Open a new terminal, then run acea doctor. The current shell's PATH is not modified."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_path_and_avoids_case_or_quote_duplicates() {
        let old = "C:\\First;%SystemRoot%\\system32";
        assert_eq!(append_path(old, "C:\\Tools").0, format!("{old};C:\\Tools"));
        assert!(!append_path("C:\\First;\"c:/TOOLS/\"", "C:\\Tools").1);
    }
    #[test]
    fn install_copy_and_replacement_are_explicit() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.exe");
        let b = d.path().join("user tools/bin/acea.exe");
        std::fs::write(&a, b"first").unwrap();
        assert!(copy_executable(&a, &b, false).unwrap());
        assert!(!copy_executable(&a, &b, false).unwrap());
        std::fs::write(&a, b"second").unwrap();
        assert!(copy_executable(&a, &b, false).is_err());
        assert_eq!(std::fs::read(&b).unwrap(), b"first");
        assert!(copy_executable(&a, &b, true).unwrap());
        assert_eq!(std::fs::read(&b).unwrap(), b"second");
    }
}
