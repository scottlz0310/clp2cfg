#Requires -Version 7.0
<#
.SYNOPSIS
scripts/verify-windows-dlls.ps1 のパース・ホワイトリスト判定ロジックの単体テスト。
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

. (Join-Path $PSScriptRoot "verify-windows-dlls.ps1")

$passed = 0
$failed = 0

function Assert-Condition {
  param(
    [string]$Name,
    [bool]$Condition,
    [string]$Message = ""
  )

  if ($Condition) {
    $script:passed++
    Write-Host "PASS: $Name"
  } else {
    $script:failed++
    Write-Host "FAIL: $Name ($Message)"
  }
}

# 1. 通常インポート＋遅延インポートのパースとヘッダ/Summary 除外
$sampleOutput = @(
  "Dump of file custom_target.dll",
  "",
  "File Type: EXECUTABLE IMAGE",
  "",
  "  Image has the following dependencies:",
  "",
  "    KERNEL32.dll",
  "    user32.dll",
  "    kernel32.dll",
  "    api-ms-win-core-synch-l1-2-0.dll",
  "",
  "  Image has the following delay load dependencies:",
  "",
  "    dwrite.dll",
  "",
  "  Summary",
  "",
  "        1000 .data",
  "        2000 .rdata"
)

$parsed = @(Get-DumpbinDependentDlls -OutputLines $sampleOutput)
$expectedParsed = @(
  "api-ms-win-core-synch-l1-2-0.dll",
  "dwrite.dll",
  "kernel32.dll",
  "user32.dll"
)
Assert-Condition -Name "通常・遅延ロード双方を正規化抽出し、ヘッダ行を誤認識しない" `
  -Condition (($parsed -join ",") -eq ($expectedParsed -join ",")) `
  -Message "actual=$($parsed -join ',')"

# 2. パース結果が空の場合は fail-closed
$emptyParsed = @(Get-DumpbinDependentDlls -OutputLines @("Dump of file empty.exe", "  Summary"))
Assert-Condition -Name "依存セクション無しは空配列を返す" `
  -Condition ($emptyParsed.Count -eq 0) `
  -Message "count=$($emptyParsed.Count)"

$emptyCheck = Test-PortableDllDependencies -DllNames $emptyParsed -System32Directory "C:\Windows\System32" -System32ExistsCheck { param($d) $true }
Assert-Condition -Name "DLL 0 件は fail-closed で不合格になる" `
  -Condition (-not $emptyCheck.IsValid) `
  -Message "IsValid=$($emptyCheck.IsValid)"

# 3. OS 標準システム DLL + api-ms-win-core-* のみなら合格
$systemDllSet = [System.Collections.Generic.HashSet[string]]::new([string[]]@(
  "kernel32.dll",
  "user32.dll",
  "dwrite.dll",
  "vcruntime140.dll",
  "msvcp140.dll",
  "ucrtbase.dll",
  "api-ms-win-crt-runtime-l1-1-0.dll"
))
$mockExists = { param([string]$dllName) $systemDllSet.Contains($dllName) }

$okResult = Test-PortableDllDependencies `
  -DllNames @("KERNEL32.dll", "user32.dll", "dwrite.dll", "api-ms-win-core-synch-l1-2-0.dll") `
  -System32Directory "C:\Windows\System32" `
  -System32ExistsCheck $mockExists
Assert-Condition -Name "OS 標準 DLL と api-ms-win-core-* のみなら合格" `
  -Condition ($okResult.IsValid -and $okResult.Violations.Count -eq 0) `
  -Message "violations=$($okResult.Violations -join '; ')"

# 4. 禁止ランタイム DLL（System32 に存在していても不合格）のテーブル駆動テスト
$forbiddenCases = @(
  "VCRUNTIME140.dll",
  "VCRUNTIME140_1.dll",
  "MSVCP140.dll",
  "msvcr120.dll",
  "vccorlib140.dll",
  "ucrtbase.dll",
  "concrt140.dll",
  "vcomp140.dll",
  "api-ms-win-crt-runtime-l1-1-0.dll",
  "api-ms-win-crt-heap-l1-1-0.dll"
)

foreach ($badDll in $forbiddenCases) {
  $res = Test-PortableDllDependencies `
    -DllNames @("kernel32.dll", $badDll) `
    -System32Directory "C:\Windows\System32" `
    -System32ExistsCheck $mockExists
  Assert-Condition -Name "禁止ランタイムを拒否: $badDll" `
    -Condition ((-not $res.IsValid) -and ($res.Violations.Count -eq 1) -and ($res.Violations[0] -like "*forbidden MSVC/UCRT runtime*")) `
    -Message "violations=$($res.Violations -join '; ')"
}

# 5. System32 に存在しないサードパーティ外部 DLL は不合格
$thirdPartyRes = Test-PortableDllDependencies `
  -DllNames @("kernel32.dll", "libssl-3-x64.dll") `
  -System32Directory "C:\Windows\System32" `
  -System32ExistsCheck $mockExists
Assert-Condition -Name "非システム外部 DLL を拒否: libssl-3-x64.dll" `
  -Condition ((-not $thirdPartyRes.IsValid) -and ($thirdPartyRes.Violations.Count -eq 1) -and ($thirdPartyRes.Violations[0] -like "*non-system external DLL*")) `
  -Message "violations=$($thirdPartyRes.Violations -join '; ')"

# 6. 正常行の後に空白を含む外部 DLL や後続行が並ぶ場合でも打ち切らず検証に失敗させる
$spacedOutput = @(
  "Dump of file app.exe",
  "",
  "File Type: EXECUTABLE IMAGE",
  "",
  "  Image has the following dependencies:",
  "",
  "    KERNEL32.dll",
  "    Vendor Helper.dll",
  "    VCRUNTIME140.dll",
  "",
  "  Image has the following delay load dependencies:",
  "",
  "    Another Helper.dll",
  "",
  "  Summary",
  "",
  "        1000 .data"
)

$spacedParsed = @(Get-DumpbinDependentDlls -OutputLines $spacedOutput)
$expectedSpaced = @(
  "another helper.dll",
  "kernel32.dll",
  "vcruntime140.dll",
  "vendor helper.dll"
)
Assert-Condition -Name "正常行の後に空白を含む外部 DLL が並んでもセクションを打ち切らず全件抽出する" `
  -Condition (($spacedParsed -join ",") -eq ($expectedSpaced -join ",")) `
  -Message "actual=$($spacedParsed -join ',')"

$spacedCheck = Test-PortableDllDependencies `
  -DllNames $spacedParsed `
  -System32Directory "C:\Windows\System32" `
  -System32ExistsCheck $mockExists
Assert-Condition -Name "空白を含む外部 DLL と後続の禁止ランタイムをすべて違反として検出する" `
  -Condition (
    (-not $spacedCheck.IsValid) -and
    ($spacedCheck.Violations.Count -eq 3) -and
    ($spacedCheck.Violations -contains "vendor helper.dll (non-system external DLL)") -and
    ($spacedCheck.Violations -contains "another helper.dll (non-system external DLL)") -and
    ($spacedCheck.Violations -contains "vcruntime140.dll (forbidden MSVC/UCRT runtime)")
  ) `
  -Message "violations=$($spacedCheck.Violations -join '; ')"

# 7. パストラバーサルや非 DLL 名は System32 存在チェック結果に関わらず拒否する
$unsafeNames = @("..\kernel32.dll", "sub\kernel32.dll", "custom module.ocx")
foreach ($unsafeName in $unsafeNames) {
  $unsafeRes = Test-PortableDllDependencies `
    -DllNames @("kernel32.dll", $unsafeName) `
    -System32Directory "C:\Windows\System32" `
    -System32ExistsCheck { param($d) $true }
  Assert-Condition -Name "不正なモジュール名を拒否: $unsafeName" `
    -Condition ((-not $unsafeRes.IsValid) -and ($unsafeRes.Violations.Count -eq 1)) `
    -Message "violations=$($unsafeRes.Violations -join '; ')"
}

# 8. 依存セクション内に解釈できない非インデント行が現れた場合は fail-closed で例外にする
$unexpectedThrown = $false
try {
  $null = Get-DumpbinDependentDlls -OutputLines @(
    "  Image has the following dependencies:",
    "    KERNEL32.dll",
    "  Unexpected Section Marker",
    "  Summary"
  )
} catch {
  $unexpectedThrown = $true
}
Assert-Condition -Name "依存セクション内の未知行は黙って打ち切らず例外を送出する" `
  -Condition $unexpectedThrown `
  -Message "thrown=$unexpectedThrown"

Write-Host ""
Write-Host "Summary: $passed passed, $failed failed"
if ($failed -gt 0) {
  exit 1
}
