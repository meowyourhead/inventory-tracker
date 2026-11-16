fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=ui/index.html");

    // Build date shown in Settings ▸ Updates (UTC, YYYY-MM-DD).
    let build_date = utc_date();
    println!("cargo:rustc-env=INVENTORY_BUILD_DATE={build_date}");

    #[cfg(windows)]
    {
        // The release version is the Cargo package version; GitHub release
        // tags (`v1.2.3`) must match it for the updater to work.
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Inventory");
        res.set("FileDescription", "Inventory");
        res.set("InternalName", "Inventory");
        res.set("OriginalFilename", "Inventory.exe");
        res.set("LegalCopyright", "Inventory");
        res.set("ProductVersion", &version);
        res.set("FileVersion", &version);
        if let Some(v) = version_number(&version) {
            res.set_version_info(winresource::VersionInfo::FILEVERSION, v);
            res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, v);
        }
        if let Err(err) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {err}");
        }
    }
}

/// `major.minor.patch` packed into the four 16-bit words Windows expects.
#[cfg(windows)]
fn version_number(value: &str) -> Option<u64> {
    let mut parts = value.split(['.', '-', '+']);
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next()?.parse().ok()?;
    let patch: u64 = parts.next()?.parse().ok()?;
    if major > 0xffff || minor > 0xffff || patch > 0xffff {
        return None;
    }
    Some((major << 48) | (minor << 32) | (patch << 16))
}

/// Today's UTC date without extra build dependencies (civil-from-days).
fn utc_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = i64::try_from(secs / 86_400).unwrap_or(0) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
