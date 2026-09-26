param([int]$Runs = 5, [int]$IdleS = 20, [string]$Server = "http://192.168.0.101:8889")
$wd = "$env:TEMP\gpuiwork"
$exe = "C:\Users\Lyam\Build\anorak-gpui-wt\anorak-gpui\target\release\anorak-gpui.exe"
$all = @()
for ($r = 0; $r -lt $Runs; $r++) {
  $log = "$wd\gb_$r.jsonl"; Remove-Item $log -ErrorAction SilentlyContinue
  $launch = [DateTimeOffset]::Now.ToUnixTimeMilliseconds()
  $p = Start-Process $exe -ArgumentList "--server",$Server,"--size","1200x600","--bench",$log -PassThru
  $res = [ordered]@{ run = $r; launch_epoch_ms = $launch }
  $deadline = (Get-Date).AddSeconds(60)
  while ((Get-Date) -lt $deadline -and -not ((Test-Path $log) -and (Select-String -Path $log -Pattern '"results_loaded"' -Quiet))) { Start-Sleep -Milliseconds 50 }
  Start-Sleep -Milliseconds 1000
  $p.Refresh(); $res.ws_after_results = $p.WorkingSet64; $res.private_after_results = $p.PrivateMemorySize64
  while ((Get-Date) -lt $deadline -and -not (Select-String -Path $log -Pattern '"done"' -Quiet)) { Start-Sleep -Milliseconds 100 }
  $p.Refresh(); $res.ws_after_script = $p.WorkingSet64; $res.private_after_script = $p.PrivateMemorySize64; $res.peak_ws = $p.PeakWorkingSet64
  $c0 = $p.TotalProcessorTime.TotalMilliseconds; Start-Sleep -Seconds $IdleS; $p.Refresh(); $c1 = $p.TotalProcessorTime.TotalMilliseconds
  $res.idle_cpu_ms_per_s = ($c1 - $c0) / $IdleS
  $res.threads = $p.Threads.Count
  Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
  $res.events = @(Get-Content $log | ForEach-Object { $_ | ConvertFrom-Json })
  $all += [pscustomobject]$res
  Write-Host ("run {0}: ws={1:N1}MB peak={2:N1}MB idle={3}" -f $r, ($res.ws_after_results/1MB), ($res.peak_ws/1MB), $res.idle_cpu_ms_per_s)
  Start-Sleep -Seconds 2
}
$all | ConvertTo-Json -Depth 8 -Compress | Out-File -Encoding utf8 "$wd\gpui_bench.json"
