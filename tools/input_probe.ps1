# Samples the HOST machine from outside while `freeviewer --inputtest` drives a
# session from the viewer side. Run it in the interactive session of the host,
# then compare sample.log with the timestamps the input test prints.
#
#   powershell -ExecutionPolicy Bypass -File tools\input_probe.ps1
#
# Columns: elapsed ms | cursor x | cursor y | NumLock state | Ctrl held | clipboard

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class FVS {
  [StructLayout(LayoutKind.Sequential)] public struct P { public int X; public int Y; }
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out P p);
  [DllImport("user32.dll")] public static extern short GetKeyState(int k);
  [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int k);
}
"@

$out = Join-Path (Get-Location) 'sample.log'
$w = New-Object IO.StreamWriter($out, $false)
$w.AutoFlush = $true
$sw = [Diagnostics.Stopwatch]::StartNew()
$w.WriteLine('   ms     x     y  numlock ctrl clipboard')
while ($sw.ElapsedMilliseconds -lt 40000) {
  $p = New-Object 'FVS+P'
  [void][FVS]::GetCursorPos([ref]$p)
  $num = [FVS]::GetKeyState(0x90) -band 1
  $ctrl = if (([FVS]::GetAsyncKeyState(0x11) -band 0x8000) -ne 0) { 1 } else { 0 }
  $clip = ''
  try { $clip = Get-Clipboard -Raw } catch { }
  if ($null -eq $clip) { $clip = '' }
  $clip = ($clip -replace "\s+", ' ')
  if ($clip.Length -gt 24) { $clip = $clip.Substring(0, 24) }
  $w.WriteLine(('{0,5} {1,5} {2,5}   {3}      {4}    {5}' -f $sw.ElapsedMilliseconds, $p.X, $p.Y, $num, $ctrl, $clip))
  Start-Sleep -Milliseconds 200
}
$w.Close()
