fn main() {
    #[cfg(windows)]
    {
        let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        let root_ico = manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("assets")
            .join("dekan.ico");
        let local_ico = manifest_dir.join("assets").join("dekan.ico");

        let icon_to_use = if root_ico.exists() {
            root_ico
        } else {
            local_ico
        };

        println!("cargo:rerun-if-changed={}", icon_to_use.display());

        let mut res = winres::WindowsResource::new();
        res.set_icon(&icon_to_use.to_string_lossy());

        // Shown in Explorer > Properties > Details. winres defaults ProductName to the crate name
        // ("dekan-app"); ProductVersion/FileVersion default to CARGO_PKG_VERSION (e.g. 1.0.0).
        // Format with two components when patch is 0 (e.g. "1.0", "1.1").
        let raw_version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
        let display_version = if let Some(stripped) = raw_version.strip_suffix(".0") {
            stripped.to_string()
        } else {
            raw_version.clone()
        };
        res.set("ProductName", "Dekan")
            .set("FileDescription", "Dekan - League of Legends skin changer")
            .set("CompanyName", "Dekan")
            .set(
                "LegalCopyright",
                "Dekan build. Original Bullet copyright (c) 2026 Isllan Toso. MIT License.",
            )
            .set("InternalName", "dekan")
            .set("OriginalFilename", "dekan.exe")
            .set("Comments", "Dekan — modified from Bullet (MIT licensed)")
            .set("ProductVersion", &display_version)
            .set("FileVersion", &display_version);

        let profile = std::env::var("PROFILE").unwrap_or_default();
        if profile == "release" {
            res.set_manifest(
                r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
<trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
        <requestedPrivileges>
            <requestedExecutionLevel level="asInvoker" uiAccess="false" />
        </requestedPrivileges>
    </security>
</trustInfo>
</assembly>
"#,
            );
        }
        // A silent failure here ships an exe without icon, version info and the asInvoker
        // manifest that keeps it unelevated, so the build must stop.
        if let Err(e) = res.compile() {
            panic!("failed to compile Windows resources (icon, version info, manifest): {e}");
        }
    }
}
