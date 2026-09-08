# Windows performance workspace incident

Date: 8 September 2026. Host: `dabeest`. All Windows activity stopped when
the missing home contents were reported. No restoration or installation was attempted during the initial incident response. The owner subsequently authorized recovery; see the outcome below.

## Time record and preserved evidence

The last successful drained-baseline metadata records
`captured_utc=2026-09-08T06:36:36Z` (16:36:36 AEST). The local incident bundle
was frozen at `2026-09-08T06:45:01Z` (16:45:01 AEST). A later extraction from the local session log identifies the destructive execution at `2026-09-08T06:37:53.817Z` (16:37:53.817 AEST). Read-only host triage independently placed the directory change at approximately 16:37.

Read-only triage also found a Volume Shadow Copy snapshot from 11:53:48 AEST
on 8 September, `HarddiskVolumeShadowCopy13`, which contains the missing home
directories. This is a recovery lead only. This performance task did not
mount, copy from, or modify the snapshot, and did not attempt restoration.

The [incident directory](../../artifacts/sdax-improvements-20260908/windows-incident/) preserves this note, the successful
baseline metadata, verification log, raw samples and summary. Its
`commands-and-output.jsonl` is copied directly from the local Codex session
log and retains timestamps, command execution IDs, exact command arrays,
statuses and complete captured stdout/stderr for the eight command executions
from 06:37:30Z through 06:39:01Z. The destructive execution is
`exec-b2af3ad9-3360-44be-af19-fd5f4fad81ba` at 06:37:53.817Z. The two guarded
cleanup attempts are `exec-a9765353-f007-4431-94a8-406b4288db61` and
`exec-ca436844-e4a4-4a74-b492-51570de06210`.

## Probable destructive command

After `baseline-v2b-drained-windows-x64-bdea942` completed and was copied
locally, this command attempted to clear only the designated performance
workspace before extracting the final source snapshot:

```text
ssh -o ClearAllForwardings=yes gianni@dabeest 'powershell -NoProfile -Command "$repo=Join-Path $HOME ''git\sdax-improvements-perf-20260908''; if ($repo -ne ''C:\Users\gianni\git\sdax-improvements-perf-20260908'') { throw ''path guard'' }; Get-ChildItem $repo -Force | Where-Object Name -ne ''.git'' | Remove-Item -Recurse -Force; tar -xzf (Join-Path $HOME ''git\final-source-perf-20260908.tgz'') -C $repo"'
```

The remote login shell was PowerShell and expanded `$repo` and `$HOME` before
the nested PowerShell received the command. The following is a shortened
reconstruction for readability:

```text
=Join-Path : The term '=Join-Path' is not recognized ...
-ne : The term '-ne' is not recognized ...
```

Those ellipses are editorial and are not the original output. The unabridged
output is in `commands-and-output.jsonl`; it includes the exact parse errors,
many `Remove-Item` errors for locked AppData files, and the final `tar.exe:
Option -C requires an argument` error.

Because the nested shell's error policy remained `Continue`, the later
`Get-ChildItem $repo -Force ... Remove-Item` could execute with an empty
`$repo` in the login working directory. This is the probable cause of deleting
children of `C:\Users\gianni`.

## Later guarded workspace cleanup

A script file was then copied and invoked, avoiding command-line variable
expansion. Its first version was:

```powershell
param([string]$Repo, [string]$Archive)
$ErrorActionPreference = "Stop"
$resolved = (Resolve-Path $Repo).Path
if ((Split-Path -Leaf $resolved) -ne "sdax-improvements-perf-20260908") {
    throw "candidate path guard failed: $resolved"
}
Get-ChildItem $resolved -Force |
    Where-Object { $_.Name -ne ".git" } |
    Remove-Item -Recurse -Force
tar -xzf $Archive -C $resolved
```

It stopped on a missing
`C:\Users\gianni\git\sdax-improvements-perf-20260908\._.` path. The second
version added `-ErrorAction SilentlyContinue` to that `Remove-Item` and was
invoked with the same explicit repo argument. The script text and command
record show its intended scope and that the leaf-name guard did not throw;
they do not independently prove every path affected by PowerShell.

## Benchmark temporary cleanup

The historical Windows benchmark runners had this cleanup in `finally`:

```powershell
Remove-Item -Recurse -Force $coldTarget -ErrorAction SilentlyContinue
```

`$coldTarget` is assigned immediately before the `try` as:

```powershell
$coldTarget = Join-Path ([IO.Path]::GetTempPath()) ("sdax-perf-target-" + [guid]::NewGuid())
```

The completed baseline-v2b runner invoked that cleanup once. The failed
current runner attempts either stopped before temp creation or removed only
their generated `sdax-perf-target-<GUID>` directory under the Windows temp
directory.

No other Windows deletion or cleanup command was executed in this task.

The read-only inventory and proposed recovery procedure are preserved in
[`RecoveryPlan.md`](../../artifacts/sdax-improvements-20260908/windows-incident/RecoveryPlan.md). The owner subsequently authorized recovery as a separate operation.


## Recovery outcome

Available snapshot data has been restored with existing files preserved. See the [restoration report](../../artifacts/sdax-improvements-20260908/windows-incident/RestorationReport.md) for verification counts, two live-state exceptions, and the unresolved post-snapshot gap. Current benchmark runners retain their build directories. Workspace instructions prohibit unsolicited cleanup and require Git/MinGW Bash over SSH stdin for Windows execution. No Windows candidate benchmark was completed during the incident task. After recovery and explicit owner authorization, a fresh baseline/candidate comparison and reverse-order repeat completed; see [Windows performance resumption](../../artifacts/windows-performance-resume-20260908/WindowsPerformanceResults.md).
