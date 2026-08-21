fn main() {
    // Link macOS frameworks for ScreenCaptureKit system audio capture
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=CoreMedia");
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    }

    // ggml's CPU backend reads the registry to detect CPU features, so it needs
    // advapi32. The library and binary get it transitively; an example target
    // links only what its own crate names, which is why `mic_chunk_probe` has
    // failed to link since it was added — and why CI has produced no installer
    // since 2026-08-09.
    #[cfg(target_os = "windows")]
    println!("cargo:rustc-link-lib=dylib=advapi32");

    tauri_build::build()
}
