//! 运行信息的采集与格式化。

/// 当前 Winc 进程的运行信息。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeInfo {
    /// Winc 的真实版本号。
    pub winc_version: &'static str,
    /// 操作系统版本；系统接口不可用时为 `None`。
    pub os_version: Option<String>,
    /// 当前程序的目标架构。
    pub architecture: &'static str,
}

/// 采集当前 Winc 进程的运行信息。
pub fn collect() -> RuntimeInfo {
    RuntimeInfo {
        winc_version: winc_version::VERSION,
        os_version: os_version(),
        architecture: std::env::consts::ARCH,
    }
}

/// 按当前界面语言格式化运行信息，供用户粘贴到问题反馈中。
pub fn report() -> String {
    let info = collect();
    let os = info
        .os_version
        .as_deref()
        .unwrap_or(winc_i18n::t!("settings-system-info-unknown"));
    format!(
        "{}: {}\n{}: {}\n{}: {}",
        winc_i18n::t!("settings-system-info-version"),
        info.winc_version,
        winc_i18n::t!("settings-system-info-os"),
        os,
        winc_i18n::t!("settings-system-info-architecture"),
        info.architecture,
    )
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    sysctl_string("kern.osproductversion").map(|version| format!("macOS {version}"))
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    windows_version()
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    linux_distribution().or_else(|| unix_kernel_version("Linux kernel"))
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn os_version() -> Option<String> {
    unix_kernel_version(&format!("{} kernel", std::env::consts::OS))
}

#[cfg(not(any(unix, windows)))]
fn os_version() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)] // sysctlbyname 是 macOS 获取系统版本的系统接口。
fn sysctl_string(name: &str) -> Option<String> {
    use std::ffi::CString;

    let name = CString::new(name).ok()?;
    let mut size = 0usize;
    // SAFETY: `name` 是以 NUL 结尾的有效 C 字符串；其余指针为空，仅查询长度。
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || size == 0
    {
        return None;
    }
    let mut bytes = vec![0u8; size];
    // SAFETY: `bytes` 有 `size` 字节；参数与上面的长度查询相同。
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    bytes.truncate(size);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    String::from_utf8(bytes).ok().filter(|value| !value.is_empty())
}

#[cfg(windows)]
#[allow(unsafe_code)] // RtlGetVersion 是 Windows 获取真实版本（含 build）的系统接口。
fn windows_version() -> Option<String> {
    #[repr(C)]
    struct OsVersionInfoExW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
        service_pack_major: u16,
        service_pack_minor: u16,
        suite_mask: u16,
        product_type: u8,
        reserved: u8,
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoExW) -> i32;
    }

    let mut info = OsVersionInfoExW {
        size: std::mem::size_of::<OsVersionInfoExW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
        service_pack_major: 0,
        service_pack_minor: 0,
        suite_mask: 0,
        product_type: 0,
        reserved: 0,
    };
    // SAFETY: `info` has the documented OSVERSIONINFOEX layout and remains valid for the call.
    let status = unsafe { RtlGetVersion(&mut info) };
    if status < 0 {
        None
    } else {
        Some(format!(
            "Windows {}.{}.{}",
            info.major, info.minor, info.build
        ))
    }
}

#[cfg(target_os = "linux")]
fn linux_distribution() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .ok()?;
    let pretty = os_release_value(&text, "PRETTY_NAME");
    let name = os_release_value(&text, "NAME");
    let version = os_release_value(&text, "VERSION_ID");
    pretty.or_else(|| match (name, version) {
        (Some(name), Some(version)) => Some(format!("{name} {version}")),
        (Some(name), None) => Some(name),
        _ => None,
    })
}

#[cfg(target_os = "linux")]
fn os_release_value(text: &str, key: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))?;
    let value = value.trim();
    let value = if let Some(value) = value.strip_prefix('"') {
        let value = value.strip_suffix('"')?;
        let mut result = String::new();
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let next = chars.next()?;
                if !matches!(next, '$' | '`' | '"' | '\\') {
                    result.push('\\');
                }
                result.push(next);
            } else {
                result.push(ch);
            }
        }
        result
    } else if let Some(value) = value.strip_prefix('\'') {
        value.strip_suffix('\'')?.to_owned()
    } else {
        value.to_owned()
    };
    (!value.is_empty()).then_some(value)
}

#[cfg(all(unix, not(target_os = "macos")))]
#[allow(unsafe_code)] // uname 是 Unix 获取内核版本的系统接口。
fn unix_kernel_version(label: &str) -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::utsname>::zeroed();
    // SAFETY: `uname` writes the complete fixed-size structure supplied by the caller.
    if unsafe { libc::uname(info.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: uname succeeded and initialized `info`.
    let info = unsafe { info.assume_init() };
    // SAFETY: uname strings are NUL-terminated within the fixed-size utsname fields.
    let release = unsafe { std::ffi::CStr::from_ptr(info.release.as_ptr()) }
        .to_str()
        .ok()?;
    (!release.is_empty()).then(|| format!("{label} {release}"))
}
