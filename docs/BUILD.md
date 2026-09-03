# Building Note67

Two ways to get an installer, and they should agree. When they disagree it is
almost always because one of them is on a different CUDA toolkit — that single
divergence cost seven CI cycles on 2026-08-21.

| | Where | Produces | When to use |
|---|---|---|---|
| **CI** | `.github/workflows/windows-build.yml` | The Vulkan installer, as a run artifact | Normally. Every push to a branch builds it |
| **Local** | `scripts/build-windows-gpu.ps1` | One installer, on your machine | Iterating on native code, or CI is unavailable |

Downloading from CI:

```powershell
gh run download --repo CyberFitz-LLC/note67 -n note67-windows-vulkan
```

**CUDA is not built on a push.** It costs about an hour against Vulkan's eight
minutes and produces an 862 MB installer against 20 MB, and it stopped earning
that once the daily driver moved to Vulkan and most transcription moved off
local Whisper. Ask for it explicitly when you want one:

```powershell
gh workflow run "Windows Build" --repo CyberFitz-LLC/note67 --ref feat/people -f cuda=true
```

Everything below still applies to that build, and the job is unchanged — this
is a matter of when it runs, not whether it works.

**The CUDA installer is ~862 MB against Vulkan's ~20 MB**, almost all of it
cuBLAS. Vulkan is GPU-accelerated on NVIDIA hardware too, so prefer it unless
you specifically need CUDA.

---

## The CUDA toolchain

Every requirement below was discovered by a build failing, and each one fails
late and describes itself badly. Read this before changing anything CUDA-shaped
in the workflow or the script.

### The toolkit version is not free to choose

**CUDA 13.x is required**, because the Windows images ship Visual Studio 18 /
MSVC 14.51 and CUDA 12.5's `nvcc` refuses a host compiler that far ahead of it.
It fails while compiling CMake's *own* compiler-identification file, so the
error names nothing in this project:

```
nvcc ... exited with code 5
CMake Error at .../CMakeDetermineCompilerId.cmake:966
```

`-allow-unsupported-compiler` is already being passed and does not rescue it.

**Keep CI and `build-windows-gpu.ps1` on the same major version.** The whole
seven-cycle episode existed because CI was on 12.5 while the local script had
moved to 13, so "it builds" meant different things in the two places.

### 13.x splits packages that 12.x bundled

The Windows installer takes a subpackage list. An invalid name aborts the
install with an exit code that names nothing; a *missing* name installs cleanly
and fails much later.

| Subpackage | Why | Symptom when missing |
|---|---|---|
| `nvcc` | The compiler driver | — |
| `crt` | `cuda_runtime.h` includes `crt/host_config.h` on line 82 | `C1083: Cannot open include file: 'crt/host_config.h'` |
| `nvvm` | Holds `cicc`, the device compiler `nvcc` shells out to | `The system cannot find the path specified` and exit 1. No mention of `cicc` or `nvvm` |
| `cudart` | Runtime | — |
| `cublas`, `cublas_dev` | ggml links them | — |
| `nvjitlink` | cuBLAS links against it from 12.4 onward | — |
| `thrust` | ggml uses it. **Still a valid name in 13.x** | — |
| `visual_studio_integration` | CMake's `enable_language(CUDA)` needs the MSBuild props | `No CUDA toolset found` |

Two traps:

- **`cccl` is not a Windows subpackage at any version.** It exists as a pip
  wheel. Passing it aborts the installer. CCCL's *headers* do ship — under
  `include/cccl/` — via `thrust`.
- **Do not drop the list and install everything.** A full install includes the
  display driver, which cannot install on a runner with no NVIDIA hardware.

The authoritative list is the subpackage table in NVIDIA's *CUDA Installation
Guide for Windows*. Read it rather than inferring names.

### Flags that nothing sets for you

```yaml
CUDAARCHS: "75-real;86-real;89-real;90-virtual"
CUDAFLAGS: "-std=c++17 -Xcompiler=/Zc:preprocessor"
```

**`CUDAARCHS`** — ggml leaves the CUDA architecture to CMake, and CMake asks the
machine. A runner has no NVIDIA card, so it falls back to a Maxwell default that
CUDA 13 dropped entirely (`nvcc fatal: Unsupported gpu architecture
'compute_52'`).

This is the GPU counterpart of `src-tauri/cmake/portable-cpu.cmake`, which
exists because ggml bakes in the build machine's CPU features and produced
installers that died with `STATUS_ILLEGAL_INSTRUCTION` elsewhere. The GPU has
the same edge, in the other direction: **on a machine that does have a GPU,
CMake detects that one card**, so an installer built at a desk is accelerated
only on that desk. `build-windows-gpu.ps1` does not set this — see Open below.

**`CUDAFLAGS`** — two separate CCCL requirements:

- `/Zc:preprocessor`: CCCL hard-`#error`s under MSVC's traditional preprocessor.
- `-std=c++17`: CUB hard-`#error`s below C++17, and nothing in whisper.cpp's
  CMake sets a CUDA standard, so `nvcc` defaults to C++14 (visible as
  `--ms_c++14` in its own invocation).

CCCL offers `CCCL_IGNORE_MSVC_TRADITIONAL_PREPROCESSOR_WARNING` and
`CCCL_IGNORE_DEPRECATED_CPP_DIALECT` to silence both. **Neither is used.** What
they suppress is a library saying it is being compiled in a way that "may lead
to unexpected compilation errors", and a quietly miscompiled CUDA kernel inside
a shipped installer is a worse outcome than a red build.

### Runtime DLLs must ship beside the exe

CUDA links its runtime dynamically, so `cudart64_*`, `cublas64_*` and
`cublasLt64_*` are staged into `src-tauri/cuda-runtime/` **before** the bundler
runs and declared as resources. Copying them afterwards leaves them outside the
installer, and the app then dies at startup on any machine without the toolkit.

**Search recursively.** CUDA 13 moved these out of `bin\` into a subdirectory, so
a flat listing finds nothing on a perfectly good toolkit and reports it as a
missing install. Deduplicate by filename — the recursive search finds several
copies.

## Diagnosing a CUDA build failure

The workflow's **"Show what the CUDA install provided"** step prints whether
`nvcc`, `cicc`, `ptxas`, `fatbinary`, `nvlink` and the key headers are present,
before anything tries to use them. Check it first: it distinguishes an
incomplete toolkit from a real compile error, which is the distinction that took
four cycles to make by hand.

If a build fails inside `whisper-rs-sys`, the useful text is well below the
`CMake Error` line — the actual `nvcc fatal` or `C1189` message is what names the
problem. `--log-failed` truncates lines, so grep the saved log rather than
reading the summary.

## Open

- **`scripts/build-windows-gpu.ps1` does not set `CUDAARCHS`.** Locally built
  installers are therefore probably compiled for the build machine's GPU alone
  and fall back to CPU elsewhere. Fixing it makes local builds slower (four
  architectures instead of one), so it is a deliberate trade rather than an
  oversight. It matters as soon as anyone other than the builder runs the
  installer.
- **CUDA CI takes ~70 minutes** against Vulkan's ~8, mostly compiling four
  architectures of ggml kernels.
