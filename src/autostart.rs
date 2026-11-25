use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE: PCWSTR = w!("Fuzzie");
const TASK_NAME: &str = "Fuzzie";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Adds or removes the per-user "start with Windows" entry. Returns success.
/// Prefers a zero-delay ONLOGON Task Scheduler task so Windows launches Fuzzie
/// immediately at logon instead of queuing it behind Explorer's HKCU\Run delay;
/// falls back to the HKCU\Run registry key if Task Scheduler is unavailable.
pub fn set(enable: bool) -> bool {
    if enable {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        if create_logon_task(&exe) {
            let _ = delete_run_key();
            return true;
        }
        set_run_key(&exe)
    } else {
        let t = delete_logon_task();
        let r = delete_run_key();
        t || r
    }
}

fn schtasks_path() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join(r"System32\schtasks.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("schtasks.exe"))
}

fn create_logon_task(exe: &Path) -> bool {
    let Some(user_id) = current_user_id() else {
        return false;
    };
    let xml = build_task_xml(&user_id, exe);
    let tmp = std::env::temp_dir().join(format!("fuzzie-autostart-{}.xml", std::process::id()));
    let mut bytes = Vec::with_capacity(2 + xml.len() * 2);
    bytes.extend_from_slice(&0xFEFFu16.to_le_bytes());
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    if std::fs::write(&tmp, &bytes).is_err() {
        return false;
    }
    let ok = Command::new(schtasks_path())
        .args(["/Create", "/F", "/TN", TASK_NAME, "/XML"])
        .arg(&tmp)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success());
    let _ = std::fs::remove_file(&tmp);
    ok
}

fn delete_logon_task() -> bool {
    Command::new(schtasks_path())
        .args(["/Delete", "/F", "/TN", TASK_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn set_run_key(exe: &Path) -> bool {
    unsafe {
        let cmd: Vec<u16> = format!("\"{}\"", exe.display())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(cmd.as_ptr().cast()),
            (cmd.len() * 2) as u32,
        )
        .is_ok()
    }
}

fn delete_run_key() -> bool {
    unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE).is_ok() }
}

/// Returns the current user's SID string (e.g. "S-1-5-21-..."), falling back to
/// DOMAIN\USER if the token query fails.
fn current_user_id() -> Option<String> {
    if let Some(sid) = current_user_sid() {
        return Some(sid);
    }
    let user = std::env::var("USERNAME").ok()?;
    match std::env::var("USERDOMAIN") {
        Ok(dom) if !dom.is_empty() => Some(format!("{dom}\\{user}")),
        _ => Some(user),
    }
}

fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = Default::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        format_sid(tu.User.Sid.0 as *const u8)
    }
}

/// Formats a binary Windows SID into its canonical "S-1-..." string form.
unsafe fn format_sid(ptr: *const u8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let rev = *ptr;
    let count = *ptr.add(1) as usize;
    if count > 15 {
        return None;
    }
    let mut auth = 0u64;
    for i in 0..6 {
        auth = (auth << 8) | (*ptr.add(2 + i) as u64);
    }
    let mut out = format!("S-{rev}-{auth}");
    for i in 0..count {
        let p = ptr.add(8 + i * 4);
        let sub = u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]);
        out.push('-');
        out.push_str(&sub.to_string());
    }
    Some(out)
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

fn build_task_xml(user_id: &str, exe: &Path) -> String {
    let uid = xml_escape(user_id);
    let cmd = xml_escape(&exe.display().to_string());
    let dir = exe
        .parent()
        .map(|p| format!("\r\n      <WorkingDirectory>{}</WorkingDirectory>", xml_escape(&p.display().to_string())))
        .unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n\
<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\r\n\
  <Triggers>\r\n\
    <LogonTrigger>\r\n\
      <Enabled>true</Enabled>\r\n\
      <UserId>{uid}</UserId>\r\n\
    </LogonTrigger>\r\n\
  </Triggers>\r\n\
  <Principals>\r\n\
    <Principal id=\"Author\">\r\n\
      <UserId>{uid}</UserId>\r\n\
      <LogonType>InteractiveToken</LogonType>\r\n\
      <RunLevel>LeastPrivilege</RunLevel>\r\n\
    </Principal>\r\n\
  </Principals>\r\n\
  <Settings>\r\n\
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\r\n\
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\r\n\
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\r\n\
    <AllowHardTerminate>false</AllowHardTerminate>\r\n\
    <StartWhenAvailable>true</StartWhenAvailable>\r\n\
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\r\n\
    <Priority>4</Priority>\r\n\
  </Settings>\r\n\
  <Actions Context=\"Author\">\r\n\
    <Exec>\r\n\
      <Command>{cmd}</Command>{dir}\r\n\
    </Exec>\r\n\
  </Actions>\r\n\
</Task>\r\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_sid_is_canonical() {
        let sid = current_user_sid().expect("current user SID");
        assert!(sid.starts_with("S-1-5-"), "unexpected SID: {sid}");
    }

    #[test]
    fn task_xml_escapes_and_sets_no_limit() {
        let xml = build_task_xml("S-1-5-21-1-2-3-1000", Path::new(r"C:\My & Apps\Fuzzie.exe"));
        assert!(xml.contains("<Command>C:\\My &amp; Apps\\Fuzzie.exe</Command>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<Priority>4</Priority>"));
    }
}
