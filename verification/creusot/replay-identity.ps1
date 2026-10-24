#Requires -Version 7.0
# The identity of the Creusot escape proof's replay record, the value the reference's claims ladder records
# on the row for this proof and a host-compliance declaration names (verification/README.md, "The replay
# identity"). It is `sha256:` and the SHA-256 of a manifest with one line per file of the record, in
# ordinal order of the file's repository-relative path:
#
#     <SHA-256 of the file's bytes, lower-case hex><two spaces><path>\n
#
# which is exactly what GNU `sha256sum` prints, so on Linux the same value is
#
#     cd <repo> && { echo src/json/escape.rs; echo verification/creusot/escape/src/lib.rs;
#       find verification/creusot/escape/verif -name proof.json; } | LC_ALL=C sort | xargs sha256sum | sha256sum
#
# The record is the proved function (src/json/escape.rs), its specification
# (verification/creusot/escape/src/lib.rs) and every replay file (verification/creusot/escape/verif/**/proof.json).
# Any change to one of them moves the identity, and the reference refuses a declaration naming an identity its
# row does not record. Files are read as committed: the repository checks text out with LF line endings.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$escape = "verification/creusot/escape"
$files = @("src/json/escape.rs", "$escape/src/lib.rs")
$files += Get-ChildItem -Path (Join-Path $root "$escape/verif") -Recurse -File -Filter proof.json |
    ForEach-Object { [IO.Path]::GetRelativePath($root, $_.FullName).Replace('\', '/') }
if ($files.Count -lt 3) { throw "the replay record under $escape/verif holds no proof.json" }
$sha = [Security.Cryptography.SHA256]::Create()
$hex = { param([byte[]]$b) ([BitConverter]::ToString($sha.ComputeHash($b)) -replace '-', '').ToLowerInvariant() }
$manifest = [Text.StringBuilder]::new()
[string[]]$sorted = $files
[Array]::Sort($sorted, [StringComparer]::Ordinal)
foreach ($f in $sorted) {
    $full = Join-Path $root $f
    if (-not (Test-Path $full -PathType Leaf)) { throw "the replay record names $f, which is not a file" }
    [void]$manifest.Append((& $hex ([IO.File]::ReadAllBytes($full))) + "  " + $f + "`n")
}
"sha256:" + (& $hex ([Text.Encoding]::UTF8.GetBytes($manifest.ToString())))
