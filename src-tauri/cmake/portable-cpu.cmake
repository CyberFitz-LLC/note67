# Pin whisper.cpp/ggml to a portable CPU baseline on Windows.
#
# ggml defaults GGML_NATIVE to ON. On MSVC that pulls in FindSIMD.cmake, which
# compiles and *runs* probe programs on the build machine and then overrides
# ggml's own `option(GGML_AVX512 ... OFF)` with ON whenever the builder's CPU
# happens to support it. A GitHub Actions runner usually does; consumer CPUs
# usually do not (Intel disabled AVX-512 from 12th gen on). The result is an
# installer that dies with STATUS_ILLEGAL_INSTRUCTION (0xc000001d) on the first
# Whisper inference, on any machine narrower than the one that built it.
#
# Upstream: https://github.com/ggml-org/whisper.cpp/issues/2928
#           https://github.com/ggml-org/whisper.cpp/issues/2554
#
# Turning GGML_NATIVE off skips that probe, so the declared defaults stand:
# AVX and AVX2 on, AVX-512 off. AVX2 is present on every CPU Windows 11
# supports, and AVX-512 buys little for Whisper anyway — upstream #2970 measures
# it as often slower.
#
# Reached via CMAKE_TOOLCHAIN_FILE (see ../.cargo/config.toml) because
# whisper-rs-sys only forwards WHISPER_* and CMAKE_* environment variables into
# its CMake invocation, so GGML_* cannot be passed directly.
#
# Deliberately Windows-only: this is where the defect is demonstrated, and the
# macOS builds are Apple Silicon where the x86 branch never runs. Guarded on the
# host rather than on MSVC because a toolchain file is read before compiler
# detection, so MSVC is not yet defined here.
#
# NOTE: this file must not set CMAKE_SYSTEM_NAME. Doing so would flip
# CMAKE_CROSSCOMPILING, which changes ggml's own defaults out from under us.

if(CMAKE_HOST_WIN32)
    set(GGML_NATIVE OFF CACHE BOOL "portable baseline: skip build-host CPU probing" FORCE)
    set(GGML_AVX    ON  CACHE BOOL "portable baseline" FORCE)
    set(GGML_AVX2   ON  CACHE BOOL "portable baseline" FORCE)
    set(GGML_FMA    ON  CACHE BOOL "portable baseline" FORCE)
    set(GGML_AVX512 OFF CACHE BOOL "portable baseline: not on consumer CPUs" FORCE)
    set(GGML_AVX512_VBMI OFF CACHE BOOL "portable baseline" FORCE)
    set(GGML_AVX512_VNNI OFF CACHE BOOL "portable baseline" FORCE)
    set(GGML_AVX512_BF16 OFF CACHE BOOL "portable baseline" FORCE)
endif()
