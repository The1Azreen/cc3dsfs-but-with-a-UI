use std::io::Read;
use std::path::{Path, PathBuf};

/// FTDI driver DLLs shipped next to the executable, taken from the archives
/// in `ftdi_lib_files` (see the notice there).
const DLLS: &[(&str, &str, &str)] = &[
    ("ftdi_lib_files/2024/01/FTD3XXLibrary_v1.3.0.9.zip", "FTD3XXLibrary/x64/DLL/FTD3XX.dll", "FTD3XX.dll"),
    ("ftdi_lib_files/2023/09/CDM-v2.12.36.4-WHQL-Certified.zip", "amd64/ftd2xx64.dll", "ftd2xx.dll"),
];

fn main() {
    slint_build::compile("ui/app.slint").expect("Slint build failed");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // Exe icon, shown in Explorer and on shortcuts.
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    if let Err(e) = resource.compile() {
        println!("cargo:warning=could not embed the exe icon: {e}");
    }

    // OUT_DIR is target/<profile>/build/<pkg>/out; the exe lives in target/<profile>.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let exe_dir = out_dir.ancestors().nth(3).expect("unexpected OUT_DIR layout").to_path_buf();
    for (archive, entry, name) in DLLS {
        println!("cargo:rerun-if-changed={archive}");
        if let Err(e) = extract(Path::new(archive), entry, &exe_dir.join(name)) {
            println!("cargo:warning=could not extract {name}: {e}");
        }
    }
}

fn extract(archive: &Path, entry: &str, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dest.exists() {
        return Ok(());
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut file = zip.by_name(entry)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    std::fs::write(dest, data)?;
    Ok(())
}
