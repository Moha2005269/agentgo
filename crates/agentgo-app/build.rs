//! Captures build facts (rustc version, profile) into compile-time env so
//! `agentgo version` reports the exact toolchain that produced the binary.

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let ver = Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=AGENTGO_BUILD_RUSTC={ver}");
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=AGENTGO_BUILD_PROFILE={profile}");
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    println!("cargo:rustc-env=AGENTGO_BUILD_TARGET={target}");

    // Chromium's Windows GPU detection expects the host executable to
    // declare modern Windows compatibility. Without this resource some GPU
    // processes fail during initialization (or Chromium falls back to very
    // slow software compositing). Feed the manifest directly to MSVC's
    // linker so the development executable and its CEF child processes all
    // inherit the same compatibility and PerMonitorV2 declarations.
    if target.contains("windows-msvc") {
        let crate_dir = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"),
        );
        let manifest = crate_dir.join("agentgo.exe.manifest");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg-bin=agentgo=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=agentgo=/MANIFESTINPUT:{}",
            manifest.display()
        );

        // The executable icon (Explorer, taskbar, Alt+Tab). Written as a
        // compiled `.res` directly so no resource compiler is required.
        let icon = crate_dir.join("assets").join("agentgo.ico");
        println!("cargo:rerun-if-changed={}", icon.display());
        let res = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"))
            .join("agentgo-icon.res");
        write_icon_resource(&icon, &res).expect("failed to build the icon resource");
        println!("cargo:rustc-link-arg-bin=agentgo={}", res.display());
        // The launcher that shortcuts start carries the same identity.
        println!("cargo:rustc-link-arg-bin=agentgo-browser={}", res.display());
    }
}

/// Appends one resource entry (numeric type and name) to a `.res` image.
fn push_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes()); // header size
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&[0xFF, 0xFF]);
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // language: neutral
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    pad4(out);
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// One node of a VERSIONINFO tree: header, UTF-16 key, value, child nodes.
fn version_node(key: &str, text: bool, value: &[u8], children: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0u8; 6];
    for unit in key.encode_utf16().chain([0]) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    pad4(&mut out);
    out.extend_from_slice(value);
    for child in children {
        pad4(&mut out);
        out.extend_from_slice(child);
    }
    let value_len = if text { value.len() / 2 } else { value.len() } as u16;
    let length = out.len() as u16;
    out[0..2].copy_from_slice(&length.to_le_bytes());
    out[2..4].copy_from_slice(&value_len.to_le_bytes());
    out[4..6].copy_from_slice(&(text as u16).to_le_bytes());
    out
}

/// The version resource. Windows takes the name it shows on the taskbar, in
/// Task Manager and in notifications from `FileDescription`; without it the
/// program is called by its file name, "agentgo".
fn version_resource() -> Vec<u8> {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let (high, low) = (major << 16 | minor, patch << 16);
    let fixed: Vec<u8> = [
        0xFEEF_04BD,
        0x0001_0000,
        high,
        low,
        high,
        low,
        0x3F,
        0,
        0x0004_0004,
        1,
        0,
        0,
        0,
    ]
    .iter()
    .flat_map(|field: &u32| field.to_le_bytes())
    .collect();
    let text = |key: &str, value: &str| {
        let wide: Vec<u8> = value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        version_node(key, true, &wide, &[])
    };
    let strings = version_node(
        "040904B0",
        true,
        &[],
        &[
            text("CompanyName", "AgentGo"),
            text("FileDescription", "AgentGo"),
            text("FileVersion", env!("CARGO_PKG_VERSION")),
            text("InternalName", "agentgo"),
            text("OriginalFilename", "agentgo.exe"),
            text("ProductName", "AgentGo"),
            text("ProductVersion", env!("CARGO_PKG_VERSION")),
        ],
    );
    let translation = version_node("Translation", false, &[0x09, 0x04, 0xB0, 0x04], &[]);
    version_node(
        "VS_VERSION_INFO",
        false,
        &fixed,
        &[
            version_node("StringFileInfo", true, &[], &[strings]),
            version_node("VarFileInfo", true, &[], &[translation]),
        ],
    )
}

/// Converts an `.ico` file into RT_ICON images plus the RT_GROUP_ICON
/// directory (resource id 1, the first icon Windows shows for the file).
fn write_icon_resource(icon: &std::path::Path, res: &std::path::Path) -> std::io::Result<()> {
    const RT_ICON: u16 = 3;
    const RT_GROUP_ICON: u16 = 14;
    let data = std::fs::read(icon)?;
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_string());
    if data.len() < 6 || data[2..4] != [1, 0] {
        return Err(bad("not an .ico file"));
    }
    let count = u16::from_le_bytes([data[4], data[5]]) as usize;
    let mut out = Vec::new();
    // Every .res file starts with an empty marker entry.
    push_resource(&mut out, 0, 0, 0, &[]);
    let mut group = vec![0, 0, 1, 0];
    group.extend_from_slice(&(count as u16).to_le_bytes());
    for index in 0..count {
        let entry = 6 + index * 16;
        let field = |at: usize| {
            data.get(entry + at..entry + at + 4)
                .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
                .ok_or_else(|| bad("truncated icon directory"))
        };
        let (size, offset) = (field(8)?, field(12)?);
        let image = data
            .get(offset..offset + size)
            .ok_or_else(|| bad("icon image outside the file"))?;
        let id = (index + 1) as u16;
        push_resource(&mut out, RT_ICON, id, 0x1010, image);
        // width, height, colors, reserved, planes, bit count, bytes in res
        group.extend_from_slice(&data[entry..entry + 12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut out, RT_GROUP_ICON, 1, 0x1030, &group);
    push_resource(&mut out, 16, 1, 0x0030, &version_resource()); // RT_VERSION
    std::fs::write(res, out)
}
