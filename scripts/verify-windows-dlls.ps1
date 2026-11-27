#Requires -Version 7.0
<#
.SYNOPSIS
Windows PE バイナリのインポート DLL を dumpbin /dependents で抽出し、
OS 標準システム DLL のみに依存していることを検証する。
#>
[CmdletBinding()]
param(
  [string]$DumpbinPath = "",

  [string]$System32Dir = "",

  [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
  [string[]]$BinaryPath = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# VC++ / UCRT 再頒布ランタイムは Windows ランナーの System32 に既定で存在し得るため、
# System32 存在チェックより先に明示的に拒否する。
$script:ForbiddenRuntimePattern = '^(vcruntime|msvcp|msvcr\d|vccorlib|ucrtbase|concrt|vcomp|api-ms-win-crt-)'
$script:CoreApiSetPattern = '^api-ms-win-core-'

function Find-DumpbinPath {
  param(
    [string]$ExplicitPath = ""
  )

  if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
    if (-not (Test-Path -LiteralPath $ExplicitPath -PathType Leaf)) {
      throw "指定された dumpbin.exe が見つかりません: $ExplicitPath"
    }
    return (Resolve-Path -LiteralPath $ExplicitPath).Path
  }

  $candidates = New-Object System.Collections.Generic.List[string]

  # 1. vswhere.exe による Visual Studio インストール検出（VS 2022 / 2026 等の 64-bit 配置にも対応）
  $vswherePaths = @(
    (Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"),
    (Join-Path $env:ProgramFiles "Microsoft Visual Studio\Installer\vswhere.exe")
  ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -Unique

  foreach ($vswhere in $vswherePaths) {
    if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
      $installPaths = & $vswhere -all -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
      if ($LASTEXITCODE -eq 0 -and $installPaths) {
        foreach ($installPath in $installPaths) {
          if ([string]::IsNullOrWhiteSpace($installPath)) {
            continue
          }
          $msvcPattern = Join-Path $installPath "VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe"
          Get-ChildItem -Path $msvcPattern -ErrorAction SilentlyContinue | ForEach-Object {
            $candidates.Add($_.FullName)
          }
        }
      }
    }
  }

  # 2. 標準インストールパス（Program Files / Program Files (x86) 双方）からのフォールバック探索
  if ($candidates.Count -eq 0) {
    $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}) |
      Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
      Select-Object -Unique
    foreach ($root in $roots) {
      $pattern = Join-Path $root "Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe"
      Get-ChildItem -Path $pattern -ErrorAction SilentlyContinue | ForEach-Object {
        $candidates.Add($_.FullName)
      }
    }
  }

  if ($candidates.Count -gt 0) {
    return ($candidates | Sort-Object -Descending | Select-Object -First 1)
  }

  # 3. PATH 上の dumpbin.exe
  $cmd = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
  if ($cmd) {
    return $cmd.Source
  }

  throw "dumpbin.exe が見つかりません（Visual Studio Build Tools の VC++ ツールが必要です）。"
}

function Get-DumpbinDependentDlls {
  param(
    [Parameter(Mandatory)]
    [AllowEmptyCollection()]
    [AllowEmptyString()]
    [AllowNull()]
    [string[]]$OutputLines
  )

  $dlls = New-Object System.Collections.Generic.List[string]
  $inSection = $false

  foreach ($rawLine in $OutputLines) {
    $line = if ($null -eq $rawLine) { "" } else { $rawLine.TrimEnd() }

    if ($line -match '^\s*Image has the following (?:delay load )?dependencies:\s*$') {
      $inSection = $true
      continue
    }

    if (-not $inSection) {
      continue
    }

    if ($line -match '^\s*Summary\s*$') {
      $inSection = $false
      continue
    }

    if ([string]::IsNullOrWhiteSpace($line)) {
      continue
    }

    # セクション内のインデントされた DLL 行のみを対象とする
    if ($line -match '^\s+([A-Za-z0-9._-]+\.dll)\s*$') {
      $dlls.Add($Matches[1].ToLowerInvariant())
    } else {
      $inSection = $false
    }
  }

  return @($dlls | Sort-Object -Unique)
}

function Test-PortableDllDependencies {
  param(
    [Parameter(Mandatory)]
    [AllowEmptyCollection()]
    [string[]]$DllNames,

    [Parameter(Mandatory)]
    [string]$System32Directory,

    [scriptblock]$System32ExistsCheck = $null
  )

  $violations = New-Object System.Collections.Generic.List[string]

  if ($DllNames.Count -eq 0) {
    $violations.Add("(no imported DLLs parsed - possible dumpbin failure or invalid PE binary)")
    return [pscustomobject]@{
      IsValid    = $false
      Dlls       = @()
      Violations = @($violations)
    }
  }

  $existsCheck = if ($null -ne $System32ExistsCheck) {
    $System32ExistsCheck
  } else {
    {
      param([string]$dllName)
      Test-Path -LiteralPath (Join-Path $System32Directory $dllName) -PathType Leaf
    }
  }

  $normalized = @($DllNames | ForEach-Object { $_.ToLowerInvariant() } | Sort-Object -Unique)

  foreach ($dll in $normalized) {
    if ($dll -match $script:ForbiddenRuntimePattern) {
      $violations.Add("$dll (forbidden MSVC/UCRT runtime)")
      continue
    }

    $inSystem32 = & $existsCheck $dll
    $isCoreApiSet = $dll -match $script:CoreApiSetPattern

    if (-not ($inSystem32 -or $isCoreApiSet)) {
      $violations.Add("$dll (non-system external DLL)")
    }
  }

  return [pscustomobject]@{
    IsValid    = ($violations.Count -eq 0)
    Dlls       = $normalized
    Violations = @($violations)
  }
}

function Invoke-VerifyWindowsDlls {
  param(
    [Parameter(Mandatory)]
    [string[]]$Paths,

    [string]$Dumpbin = "",

    [string]$System32 = ""
  )

  if ($Paths.Count -eq 0) {
    throw "検査対象のバイナリパスを 1 つ以上指定してください。"
  }

  $resolvedDumpbin = Find-DumpbinPath -ExplicitPath $Dumpbin
  $resolvedSystem32 = if ([string]::IsNullOrWhiteSpace($System32)) {
    Join-Path $env:SystemRoot "System32"
  } else {
    $System32
  }

  if (-not (Test-Path -LiteralPath $resolvedSystem32 -PathType Container)) {
    throw "System32 ディレクトリが見つかりません: $resolvedSystem32"
  }

  Write-Host "Using dumpbin: $resolvedDumpbin"

  $hasFailure = $false
  foreach ($exe in $Paths) {
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
      throw "検査対象のバイナリが見つかりません: $exe"
    }

    $output = & $resolvedDumpbin /nologo /dependents $exe 2>&1
    if ($LASTEXITCODE -ne 0) {
      $detail = ($output | Out-String).Trim()
      throw "dumpbin /dependents が失敗しました (exit=$LASTEXITCODE, target=$exe): $detail"
    }

    $lines = @($output | ForEach-Object { $_.ToString() })
    $dlls = @(Get-DumpbinDependentDlls -OutputLines $lines)
    $result = Test-PortableDllDependencies -DllNames $dlls -System32Directory $resolvedSystem32

    if (-not $result.IsValid) {
      $hasFailure = $true
      $joined = $result.Violations -join "`n - "
      Write-Error "Non-portable DLL dependencies detected in ${exe}:`n - $joined" -ErrorAction Continue
      continue
    }

    Write-Host "OK: $exe imports only portable OS system DLLs ($($result.Dlls.Count) unique DLLs): $($result.Dlls -join ', ')"
  }

  if ($hasFailure) {
    exit 1
  }
}

# dot-source された場合は関数定義のみ公開し、直接実行された場合のみ検証を実行する
if ($MyInvocation.InvocationName -ne '.') {
  Invoke-VerifyWindowsDlls -Paths $BinaryPath -Dumpbin $DumpbinPath -System32 $System32Dir
}
