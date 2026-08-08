<#
.SYNOPSIS
    Build a Note67 Windows installer with a GPU backend.

.DESCRIPTION
    Checks prerequisites, works around the CUDA/Visual Studio integration gap,
    builds, and reports where the installer landed.

    Run from the repo root:
        .\scripts\build-windows-gpu.ps1              # CUDA (default)
        .\scripts\build-windows-gpu.ps1 -Backend vulkan

.NOTES
    Prerequisites, all one-time:
      winget install Rustlang.Rustup
      winget install OpenJS.NodeJS.LTS
      winget install LLVM.LLVM             # bindgen needs libclang.dll
      winget install Kitware.CMake         # whisper.cpp is a CMake project
      winget install Microsoft.VisualStudio.2022.BuildTools `
        --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
      winget install Nvidia.CUDA          # CUDA builds only
      # Vulkan builds only: https://vulkan.lunarg.com/sdk/home#windows
#>
[CmdletBinding()]
param(
    [ValidateSet('cuda', 'vulkan')]
    [string]$Backend = 'cuda'
)

$ErrorActionPreference = 'Stop'

function Require-Command {
    param([string]$Name, [string]$Hint)
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name not found on PATH. $Hint"
    }
    Write-Host "  ok  $Name" -ForegroundColor DarkGreen
}

# The script lives in scripts/, so the repo root is its parent.
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
Write-Host "Repo: $repo`n" -ForegroundColor Cyan

Write-Host "Checking prerequisites..." -ForegroundColor Cyan
Require-Command cargo "Install with: winget install Rustlang.Rustup (then restart the shell)"
Require-Command npm   "Install with: winget install OpenJS.NodeJS.LTS (then restart the shell)"
# whisper.cpp builds through CMake. Preinstalled on GitHub's runners, so CI
# never surfaces its absence.
Require-Command cmake "Install with: winget install Kitware.CMake (then restart the shell)"

# The MSVC linker is what actually matters, and it is not on PATH outside a
# developer shell — so look for the installation rather than the executable.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vs) { Write-Host "  ok  MSVC toolchain ($vs)" -ForegroundColor DarkGreen }
    else { throw "Visual Studio C++ build tools not found. See the prerequisites in this script's header." }
} else {
    Write-Warning "vswhere.exe not found; cannot verify MSVC. Continuing — the build will fail loudly if it is missing."
}

# whisper-rs-sys runs bindgen over the whisper headers, and bindgen loads
# libclang.dll at build time. GitHub's runners ship LLVM preinstalled, so CI
# never surfaces this; a clean workstation does — 20 minutes into the build,
# which is why it is checked here instead.
function Get-LibclangMajor {
    param([string]$Dir)
    $dll = Join-Path $Dir "libclang.dll"
    if (-not (Test-Path $dll)) { return $null }
    $fv = (Get-Item $dll).VersionInfo.FileVersion
    if ($fv -match '^(\d+)\.') { return [int]$Matches[1] }
    return 0
}

if (-not $env:LIBCLANG_PATH -or -not (Test-Path (Join-Path $env:LIBCLANG_PATH "libclang.dll"))) {
    $candidates = @("C:\LLVM18\bin", "${env:ProgramFiles}\LLVM\bin")
    if ($vs) {
        # Visual Studio ships clang under the C++ Clang tools component.
        $candidates += @("$vs\VC\Tools\Llvm\x64\bin", "$vs\VC\Tools\Llvm\bin")
    }
    $found = $candidates | ForEach-Object { [pscustomobject]@{ Dir = $_; Major = (Get-LibclangMajor $_) } } |
             Where-Object { $null -ne $_.Major }
    if (-not $found) {
        throw "libclang.dll not found. Install with: winget install LLVM.LLVM (then restart the shell), or set LIBCLANG_PATH to a directory containing libclang.dll."
    }
    # Prefer a version bindgen 0.69 can actually parse. Taking the newest is
    # what produces opaque bindings and seventy misleading compile errors.
    $usable = $found | Where-Object { $_.Major -lt 20 } | Select-Object -First 1
    $pick = if ($usable) { $usable } else { $found | Select-Object -First 1 }
    $env:LIBCLANG_PATH = $pick.Dir
    Write-Host "  ok  libclang $($pick.Major) ($($pick.Dir))" -ForegroundColor DarkGreen
} else {
    Write-Host "  ok  libclang ($env:LIBCLANG_PATH)" -ForegroundColor DarkGreen
}

$clangDll = Join-Path $env:LIBCLANG_PATH "libclang.dll"
$clangMajor = $null
if (Test-Path $clangDll) {
    # Read the DLL's own version rather than shelling out to clang.exe, which
    # is not always installed beside it (Visual Studio's clang directory often
    # has no clang.exe at all).
    $fv = (Get-Item $clangDll).VersionInfo.FileVersion
    if ($fv -match '^(\d+)\.') { $clangMajor = [int]$Matches[1] }
    Write-Host "      libclang $fv" -ForegroundColor DarkGray
}
if ($clangMajor -and $clangMajor -ge 20) {
    throw @"
libclang $clangMajor is too new for bindgen 0.69, which whisper-rs-sys pins.

bindgen only warns about this, then emits opaque structs, so the build fails
much later with dozens of "no field ... on type whisper_full_params" errors.

Fix:
  Invoke-WebRequest https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8/LLVM-18.1.8-win64.exe -OutFile "`$env:TEMP\llvm18.exe"
  Start-Process -Wait "`$env:TEMP\llvm18.exe" -ArgumentList "/S","/D=C:\LLVM18"
  `$env:LIBCLANG_PATH = "C:\LLVM18\bin"
  cargo clean -p whisper-rs-sys -p whisper-rs --manifest-path src-tauri\Cargo.toml
"@
}

if ($Backend -eq 'cuda') {
    if (-not $env:CUDA_PATH -or -not (Test-Path "$env:CUDA_PATH\bin\nvcc.exe")) {
        throw "CUDA toolkit not found (CUDA_PATH unset or missing bin\nvcc.exe). Install: winget install Nvidia.CUDA, then restart the shell."
    }
    Write-Host "  ok  CUDA ($env:CUDA_PATH)" -ForegroundColor DarkGreen
    # CUDA_PATH does not necessarily point at the newest toolkit installed, and
    # nvcc rejects host compilers it does not know. VS 2022 is the mainline
    # target for 12.x and 13.x alike.
    $nvccVer = (& "$env:CUDA_PATH\bin\nvcc.exe" --version 2>&1 | Select-String 'release ([\d.]+)').Matches.Groups[1].Value
    if ($nvccVer) { Write-Host "      nvcc release $nvccVer" -ForegroundColor DarkGray }

    # The toolkit ships MSBuild integration but does not register it with Visual
    # Studio, so CMake's enable_language(CUDA) fails with "No CUDA toolset
    # found". Copying the props/targets in is the standard fix. Needs admin,
    # since it writes under Program Files.
    $src = "$env:CUDA_PATH\extras\visual_studio_integration\MSBuildExtensions"
    if (Test-Path $src) {
        # Search the install vswhere reported first, then BOTH Program Files
        # roots. Build Tools commonly lands under "Program Files (x86)" while a
        # full Visual Studio lands under "Program Files"; looking in only one of
        # them finds nothing and — worse — finds it silently.
        $roots = @()
        if ($vs) { $roots += $vs }
        $roots += @("${env:ProgramFiles}\Microsoft Visual Studio", "${env:ProgramFiles(x86)}\Microsoft Visual Studio")
        $targets = $roots | Where-Object { $_ -and (Test-Path $_) } | ForEach-Object {
            Get-ChildItem $_ -Recurse -Directory -Filter BuildCustomizations -ErrorAction SilentlyContinue
        } | Sort-Object FullName -Unique

        if (-not $targets) {
            throw "No VC BuildCustomizations directory found under: $($roots -join '; '). Without it CMake fails with 'No CUDA toolset found'."
        }
        $registered = 0
        foreach ($t in $targets) {
            if (Test-Path (Join-Path $t.FullName "CUDA*.props")) {
                Write-Host "  ok  CUDA already registered in $($t.FullName)" -ForegroundColor DarkGreen
                $registered++; continue
            }
            try {
                Copy-Item "$src\*" $t.FullName -Force -ErrorAction Stop
                Write-Host "  ok  registered CUDA into $($t.FullName)" -ForegroundColor DarkGreen
                $registered++
            } catch {
                Write-Warning "Could not write to $($t.FullName): $($_.Exception.Message)"
            }
        }
        if ($registered -eq 0) {
            throw "Found $($targets.Count) BuildCustomizations director$(if($targets.Count -eq 1){'y'}else{'ies'}) but registered none — re-run this script in an elevated shell."
        }
    } else {
        Write-Warning "CUDA VS integration not found at $src; the build may fail with 'No CUDA toolset found'."
    }
} else {
    if (-not $env:VULKAN_SDK) {
        $found = Get-ChildItem "C:\VulkanSDK" -Directory -ErrorAction SilentlyContinue |
                 Sort-Object Name -Descending | Select-Object -First 1
        if ($found) { $env:VULKAN_SDK = $found.FullName }
    }
    if (-not $env:VULKAN_SDK) { throw "Vulkan SDK not found. Install from https://vulkan.lunarg.com/sdk/home#windows" }
    Write-Host "  ok  Vulkan SDK ($env:VULKAN_SDK)" -ForegroundColor DarkGreen
}

# whisper-rs-sys does not declare rerun-if-env-changed for LIBCLANG_PATH, so
# cargo considers the crate fresh even when a different libclang would generate
# different bindings. Pointing at a new LLVM therefore changes nothing until the
# build directory is removed, and the failure looks identical — 71 "no field ...
# on type whisper_full_params" errors from cached, opaque bindings.
$stamp = Join-Path $repo "src-tauri\target\.libclang-used"
$previous = if (Test-Path $stamp) { (Get-Content $stamp -Raw).Trim() } else { $null }
if ($previous -and $previous -ne $env:LIBCLANG_PATH) {
    Write-Host "`nlibclang changed since the last build:" -ForegroundColor Yellow
    Write-Host "  was: $previous"
    Write-Host "  now: $env:LIBCLANG_PATH"
    Write-Host "Removing cached whisper-rs-sys bindings so they are regenerated." -ForegroundColor Yellow
    Get-ChildItem "$repo\src-tauri\target" -Recurse -Directory -Filter "whisper-rs-sys-*" -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item $_.FullName -Recurse -Force -ErrorAction SilentlyContinue }
}
New-Item -ItemType Directory -Force -Path (Split-Path $stamp) | Out-Null
Set-Content -Path $stamp -Value $env:LIBCLANG_PATH

Write-Host "`nInstalling npm dependencies..." -ForegroundColor Cyan
npm ci
if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }

# CUDA links its runtime dynamically, so those DLLs have to ship beside the exe
# or the app dies at startup. Stage them before the bundler runs.
$bundleConfig = '{"bundle":{"createUpdaterArtifacts":false}}'
if ($Backend -eq 'cuda') {
    $stage = Join-Path $repo "src-tauri\cuda-runtime"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    $dlls = Get-ChildItem "$env:CUDA_PATH\bin" -Filter *.dll |
            Where-Object { $_.Name -match '^(cudart64|cublas64|cublasLt64)_' }
    if (-not $dlls) { throw "No cudart/cublas DLLs found in $env:CUDA_PATH\bin" }
    foreach ($d in $dlls) { Copy-Item $d.FullName $stage -Force; Write-Host "  staged $($d.Name)" }
    $bundleConfig = '{"bundle":{"createUpdaterArtifacts":false,"resources":{"cuda-runtime/*.dll":"./"}}}'
}

Write-Host "`nBuilding ($Backend). First build takes 15-40 minutes — whisper.cpp and the GPU backend compile from source.`n" -ForegroundColor Cyan
npm run tauri build -- --features "gpu-$Backend" --config $bundleConfig
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

Write-Host "`nVerifying the build is portable and GPU-enabled..." -ForegroundColor Cyan
$caches = Get-ChildItem "src-tauri\target" -Recurse -Filter CMakeCache.txt -ErrorAction SilentlyContinue |
          Where-Object { $_.FullName -like "*whisper-rs-sys*" }
$backendVar = if ($Backend -eq 'vulkan') { 'GGML_VULKAN' } else { 'GGML_CUDA' }
$sawBackend = $false
foreach ($c in $caches) {
    $lines = Get-Content $c.FullName
    ($lines | Select-String "^(GGML_NATIVE|$backendVar):BOOL=") | ForEach-Object { Write-Host "  $($_.Line)" }
    if (($lines | Select-String "^$backendVar`:BOOL=ON")) { $sawBackend = $true }
}
if (-not $sawBackend) {
    Write-Warning "$backendVar was never ON — this build would silently run on CPU."
}

$installers = Get-ChildItem "src-tauri\target\release\bundle" -Recurse -Include *.exe,*.msi -ErrorAction SilentlyContinue
Write-Host "`nInstallers:" -ForegroundColor Green
foreach ($i in $installers) { Write-Host "  $($i.FullName)  ($([math]::Round($i.Length/1MB,1)) MB)" }
Write-Host "`nAfter installing, confirm the GPU is in use:" -ForegroundColor Cyan
Write-Host '  Start-Process "$env:LOCALAPPDATA\Note67\Note67.exe" -RedirectStandardError "$env:USERPROFILE\note67-err.txt" -RedirectStandardOutput "$env:USERPROFILE\note67-out.txt"'
Write-Host '  Select-String "$env:USERPROFILE\note67-err.txt" -Pattern "use gpu|register_backend|register_device"'
