# Posts a mouse move to a window (client coordinates in physical pixels) without moving the
# real cursor: powershell -File tools\hover.ps1 -ProcessId <pid> -X <x> -Y <y>
param([int]$ProcessId, [int]$X, [int]$Y)
Add-Type @"
using System; using System.Runtime.InteropServices;
public class HV { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l); }
"@
$h = (Get-Process -Id $ProcessId).MainWindowHandle
$lp = [IntPtr](($Y -shl 16) -bor ($X -band 0xFFFF))
[HV]::PostMessage($h, 0x0200, [IntPtr]::Zero, $lp) | Out-Null
