# AgentGo native lane: Windows UI Automation for other programs' windows.
# Reads one JSON request on stdin ({op, ...}) and writes one JSON reply.
# Elements are addressed by window handle plus a path of child indexes in
# the UI Automation control view ("0.2.1"), or by name/automationId/type.
$ErrorActionPreference = 'Stop'
# Replies are UTF-8; the console's legacy code page would mangle names and text.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::InputEncoding = New-Object System.Text.UTF8Encoding $false
$OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, UIAutomationClientsideProviders, System.Windows.Forms
# Without the client-side providers, classic Win32 controls show as bare panes.
try { [System.Windows.Automation.ClientSettings]::RegisterClientSideProviderAssembly([UIAutomationClientsideProviders.UIAutomationClientSideProviders].Assembly.GetName()) } catch {}
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public class AgN {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr h, out int p);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int c, uint f);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint c);
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint c, uint t);
  [DllImport("user32.dll")] public static extern short VkKeyScan(char c);
  [DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint a, uint p, ref int v, uint w);
  [DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint a, uint p, ref HIGHCONTRAST v, uint w);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct HIGHCONTRAST { public uint cbSize; public uint dwFlags; public IntPtr s; }
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  public static IntPtr Lp(int x, int y) { return (IntPtr)((y << 16) | (x & 0xFFFF)); }
}
'@
# Physical pixels everywhere, as UI Automation reports them.
[void][AgN]::SetProcessDpiAwarenessContext([IntPtr](-4))
$A = [System.Windows.Automation.AutomationElement]
$req = [Console]::In.ReadToEnd() | ConvertFrom-Json
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker

function Fail($code, $message) { @{ error = @{ code = $code; message = $message } } | ConvertTo-Json -Compress; exit 0 }
function Num($v) { if ([double]::IsInfinity($v) -or [double]::IsNaN($v)) { 0 } else { [int]$v } }
function Rect($r) { @{ x = (Num $r.X); y = (Num $r.Y); width = (Num $r.Width); height = (Num $r.Height) } }
function Text([IntPtr]$h, $class) { $s = New-Object System.Text.StringBuilder 512; if ($class) { [void][AgN]::GetClassName($h, $s, 512) } else { [void][AgN]::GetWindowText($h, $s, 512) }; $s.ToString() }

function WinInfo([IntPtr]$h) {
  $r = New-Object AgN+RECT; [void][AgN]::GetWindowRect($h, [ref]$r); $p = 0; [void][AgN]::GetWindowThreadProcessId($h, [ref]$p)
  $proc = try { (Get-Process -Id $p).ProcessName } catch { '' }
  @{ hwnd = [int64]$h; title = (Text $h $false); class = (Text $h $true); pid = $p; process = $proc
     visible = [AgN]::IsWindowVisible($h); minimized = [AgN]::IsIconic($h); maximized = [AgN]::IsZoomed($h)
     bounds = @{ x = $r.L; y = $r.T; width = $r.R - $r.L; height = $r.B - $r.T } }
}

function Windows {
  $list = New-Object System.Collections.ArrayList
  $cb = [AgN+EnumProc]{ param($h, $l)
    # Top-level, visible, unowned windows with a title: what a person sees.
    if ([AgN]::IsWindowVisible($h) -and [AgN]::GetWindow($h, 4) -eq [IntPtr]::Zero -and (Text $h $false)) { [void]$list.Add($h) }
    $true }
  [void][AgN]::EnumWindows($cb, [IntPtr]::Zero)
  $list
}

function Window {
  if ($req.hwnd) { return [IntPtr][int64]$req.hwnd }
  $want = "$($req.window)"
  if (-not $want) { Fail 'invalid.params' 'hwnd or window (part of a title or process name) is required' }
  $all = @(Windows | ForEach-Object { WinInfo $_ })
  $hit = @($all | Where-Object { $_.title -eq $want }) + @($all | Where-Object { $_.title -like "*$want*" -or $_.process -eq $want })
  if ($hit.Count) { return [IntPtr][int64]$hit[0].hwnd }
  Fail 'target.stale' "no window matches '$want'"
}

function Pattern($el, $p) { $o = $null; if ($el.TryGetCurrentPattern($p, [ref]$o)) { $o } else { $null } }
function Patterns($el) { ,@($el.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName -replace 'PatternIdentifiers.Pattern', '' -replace 'Pattern$', '' }) }
function Value($el) {
  $v = Pattern $el ([System.Windows.Automation.ValuePattern]::Pattern); if ($v) { return $v.Current.Value }
  $t = Pattern $el ([System.Windows.Automation.TogglePattern]::Pattern); if ($t) { return "$($t.Current.ToggleState)" }
  $r = Pattern $el ([System.Windows.Automation.RangeValuePattern]::Pattern); if ($r) { return "$($r.Current.Value)" }
  $s = Pattern $el ([System.Windows.Automation.SelectionPattern]::Pattern)
  if ($s) { return (@($s.Current.GetSelection() | ForEach-Object { $_.Current.Name }) -join ', ') }
  $x = Pattern $el ([System.Windows.Automation.TextPattern]::Pattern); if ($x) { return $x.DocumentRange.GetText(20000) }
  $null
}
function Describe($el, $path) {
  $c = $el.Current
  $d = @{ path = $path; type = ($c.ControlType.ProgrammaticName -replace 'ControlType\.', ''); name = $c.Name
          automationId = $c.AutomationId; class = $c.ClassName; enabled = $c.IsEnabled; bounds = (Rect $c.BoundingRectangle)
          patterns = (Patterns $el) }
  $v = Value $el; if ($null -ne $v -and "$v" -ne '') { $d.value = "$v".Substring(0, [Math]::Min(500, "$v".Length)) }
  $d
}
function Children($el) { $out = @(); $c = $walker.GetFirstChild($el); while ($c) { $out += $c; $c = $walker.GetNextSibling($c) }; ,$out }

function Walk($el, $path, $depth, $list) {
  [void]$list.Add((Describe $el $path))
  if ($depth -le 0 -or $list.Count -ge 400) { return }
  $i = 0; foreach ($c in (Children $el)) { Walk $c "$path.$i" ($depth - 1) $list; $i++ }
}

function Matches($el) {
  $c = $el.Current
  (-not $req.name -or $c.Name -like "*$($req.name)*") -and (-not $req.automationId -or $c.AutomationId -eq $req.automationId) -and
  (-not $req.type -or ($c.ControlType.ProgrammaticName -replace 'ControlType\.', '') -eq $req.type) -and (-not $req.class -or $c.ClassName -like "*$($req.class)*")
}
function Search($el, $path, $depth, $list) {
  if ($path -ne '0' -and (Matches $el)) { [void]$list.Add(@{ el = $el; path = $path }) }
  if ($depth -le 0 -or $list.Count -ge 50) { return }
  $i = 0; foreach ($c in (Children $el)) { Search $c "$path.$i" ($depth - 1) $list; $i++ }
}

# The element a request names: a path, or the first match of name/automationId/type.
function Target {
  $root = $A::FromHandle((Window))
  if ($req.path) {
    $el = $root
    foreach ($i in ("$($req.path)" -split '\.' | Select-Object -Skip 1)) {
      $kids = Children $el; if ([int]$i -ge $kids.Count) { Fail 'target.stale' "path $($req.path) no longer exists; call native.find again" }
      $el = $kids[[int]$i]
    }
    return @{ el = $el; path = "$($req.path)" }
  }
  if (-not ($req.name -or $req.automationId -or $req.type -or $req.class)) { return @{ el = $root; path = '0' } }
  $found = New-Object System.Collections.ArrayList; Search $root '0' 12 $found
  if ($found.Count -eq 0) { Fail 'target.stale' 'no element matches' }
  $found[0]
}

function Center($el) { $r = $el.Current.BoundingRectangle; @{ x = [int]($r.X + $r.Width / 2); y = [int]($r.Y + $r.Height / 2) } }
# A click delivered as window messages: the window is not activated and the
# person's cursor does not move.
function PostClick($el, $double) {
  $p = Center $el; $pt = New-Object AgN+POINT; $pt.X = $p.x; $pt.Y = $p.y
  # The element's own window (or its nearest ancestor's), never whatever
  # happens to lie on top at that point of the screen.
  $owner = $el; $h = [IntPtr]::Zero
  while ($owner -and $h -eq [IntPtr]::Zero) { $h = [IntPtr]$owner.Current.NativeWindowHandle; if ($h -eq [IntPtr]::Zero) { $owner = $walker.GetParent($owner) } }
  if ($h -eq [IntPtr]::Zero) { Fail 'target.blocked' 'the element has no window to send the click to' }
  [void][AgN]::ScreenToClient($h, [ref]$pt); $lp = [AgN]::Lp($pt.X, $pt.Y)
  [void][AgN]::PostMessage($h, 0x201, [IntPtr]1, $lp); [void][AgN]::PostMessage($h, 0x202, [IntPtr]0, $lp)
  if ($double) { [void][AgN]::PostMessage($h, 0x203, [IntPtr]1, $lp); [void][AgN]::PostMessage($h, 0x202, [IntPtr]0, $lp) }
  'messages'
}
function Click($el) {
  $inv = Pattern $el ([System.Windows.Automation.InvokePattern]::Pattern); if ($inv) { $inv.Invoke(); return 'invoke' }
  $tg = Pattern $el ([System.Windows.Automation.TogglePattern]::Pattern); if ($tg) { $tg.Toggle(); return 'toggle' }
  $si = Pattern $el ([System.Windows.Automation.SelectionItemPattern]::Pattern); if ($si) { $si.Select(); return 'select' }
  $ec = Pattern $el ([System.Windows.Automation.ExpandCollapsePattern]::Pattern); if ($ec) { $ec.Expand(); return 'expand' }
  PostClick $el $false
}

$VK = @{ enter = 0x0D; return = 0x0D; tab = 0x09; escape = 0x1B; esc = 0x1B; backspace = 0x08; delete = 0x2E; space = 0x20
  up = 0x26; down = 0x28; left = 0x25; right = 0x27; home = 0x24; end = 0x23; pageup = 0x21; pagedown = 0x22
  ctrl = 0x11; control = 0x11; shift = 0x10; alt = 0x12; win = 0x5B; insert = 0x2D }
1..12 | ForEach-Object { $VK["f$_"] = 0x6F + $_ }
function KeyCode($k) { $n = "$k".ToLower(); if ($VK.ContainsKey($n)) { return $VK[$n] }; if ("$k".Length -eq 1) { return ([AgN]::VkKeyScan([char]"$k") -band 0xFF) }; Fail 'invalid.params' "unknown key $k" }

function Dialogs {
  @(Windows | Where-Object { (Text $_ $true) -eq '#32770' } | ForEach-Object { WinInfo $_ })
}

$out = switch ($req.op) {
  'windows' { @{ windows = @(Windows | ForEach-Object { WinInfo $_ }) } }
  'monitors' { @{ monitors = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { @{ name = $_.DeviceName; primary = $_.Primary; bounds = (Rect $_.Bounds); workArea = (Rect $_.WorkingArea) } }) } }
  'bounds' {
    $h = Window
    if ($null -ne $req.x -or $null -ne $req.width) {
      $i = WinInfo $h; $b = $i.bounds
      $x = if ($null -ne $req.x) { [int]$req.x } else { $b.x }; $y = if ($null -ne $req.y) { [int]$req.y } else { $b.y }
      $w = if ($null -ne $req.width) { [int]$req.width } else { $b.width }; $hh = if ($null -ne $req.height) { [int]$req.height } else { $b.height }
      if ([AgN]::IsZoomed($h) -or [AgN]::IsIconic($h)) { [void][AgN]::ShowWindow($h, 4) }
      [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $x, $y, $w, $hh, 0x0014)  # NOZORDER | NOACTIVATE
    }
    WinInfo $h
  }
  'snap' {
    $h = Window; $s = [System.Windows.Forms.Screen]::FromHandle($h).WorkingArea
    switch ("$($req.side)") {
      'left' { [void][AgN]::ShowWindow($h, 4); [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $s.X, $s.Y, [int]($s.Width / 2), $s.Height, 0x14) }
      'right' { [void][AgN]::ShowWindow($h, 4); [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $s.X + [int]($s.Width / 2), $s.Y, [int]($s.Width / 2), $s.Height, 0x14) }
      'top' { [void][AgN]::ShowWindow($h, 4); [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $s.X, $s.Y, $s.Width, [int]($s.Height / 2), 0x14) }
      'bottom' { [void][AgN]::ShowWindow($h, 4); [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $s.X, $s.Y + [int]($s.Height / 2), $s.Width, [int]($s.Height / 2), 0x14) }
      'fill' { [void][AgN]::ShowWindow($h, 4); [void][AgN]::SetWindowPos($h, [IntPtr]::Zero, $s.X, $s.Y, $s.Width, $s.Height, 0x14) }
      'minimize' { [void][AgN]::ShowWindow($h, 7) }   # SW_SHOWMINNOACTIVE
      'restore' { [void][AgN]::ShowWindow($h, 4) }    # SW_SHOWNOACTIVATE
      default { Fail 'invalid.params' 'side: left, right, top, bottom, fill, minimize or restore' }
    }
    WinInfo $h
  }
  'focus' { $h = Window; [void][AgN]::ShowWindow($h, 9); $ok = [AgN]::SetForegroundWindow($h); @{ focused = $ok; window = (WinInfo $h) } }
  'tree' { $t = Target; $list = New-Object System.Collections.ArrayList; Walk $t.el $t.path ([int]($(if ($req.depth) { $req.depth } else { 3 }))) $list; @{ elements = @($list); truncated = ($list.Count -ge 400) } }
  'find' { $root = $A::FromHandle((Window)); $found = New-Object System.Collections.ArrayList; Search $root '0' 12 $found; @{ matches = @($found | ForEach-Object { Describe $_.el $_.path }) } }
  'read' { $t = Target; Describe $t.el $t.path }
  'click' { $t = Target; $how = Click $t.el; @{ clicked = $t.path; how = $how; name = $t.el.Current.Name } }
  'double_click' {
    $t = Target
    if ($req.real) {
      # Real input reaches only the top window at that point: the window comes
      # forward for the click, then the person's window and cursor return.
      $h = Window; $before = [AgN]::GetForegroundWindow(); $cur = New-Object AgN+POINT; [void][AgN]::GetCursorPos([ref]$cur)
      [void][AgN]::SetForegroundWindow($h); Start-Sleep -Milliseconds 120; $p = Center $t.el; [void][AgN]::SetCursorPos($p.x, $p.y)
      1..2 | ForEach-Object { [AgN]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero); [AgN]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero) }
      Start-Sleep -Milliseconds 120; [void][AgN]::SetCursorPos($cur.X, $cur.Y); if ($before -ne $h) { [void][AgN]::SetForegroundWindow($before) }
      @{ clicked = $t.path; how = 'input'; double = $true }
    } else { $how = PostClick $t.el $true; @{ clicked = $t.path; how = $how; double = $true } }
  }
  'type' {
    $t = Target; $v = Pattern $t.el ([System.Windows.Automation.ValuePattern]::Pattern)
    if ($v -and -not $v.Current.IsReadOnly) {
      $text = if ($req.append) { $v.Current.Value + $req.text } else { "$($req.text)" }
      $v.SetValue($text); @{ typed = $t.path; how = 'value'; value = $v.Current.Value }
    } else {
      $h = [IntPtr]$t.el.Current.NativeWindowHandle; if ($h -eq [IntPtr]::Zero) { Fail 'target.blocked' 'the element takes no text' }
      foreach ($ch in "$($req.text)".ToCharArray()) { [void][AgN]::PostMessage($h, 0x102, [IntPtr][int]$ch, [IntPtr]1) }
      @{ typed = $t.path; how = 'messages' }
    }
  }
  'key' {
    $t = Target; $h = [IntPtr]$t.el.Current.NativeWindowHandle; if ($h -eq [IntPtr]::Zero) { $h = Window }
    $code = KeyCode $req.key
    [void][AgN]::PostMessage($h, 0x100, [IntPtr]$code, [IntPtr]1); [void][AgN]::PostMessage($h, 0x101, [IntPtr]$code, [IntPtr]0xC0000001)
    @{ key = $req.key; window = [int64]$h }
  }
  'hotkey' {
    # Shortcuts read the keyboard state, which only the foreground window has:
    # the window comes forward for the keys, then the person's window returns.
    $h = Window; $before = [AgN]::GetForegroundWindow(); $codes = @($req.keys | ForEach-Object { KeyCode $_ })
    [void][AgN]::ShowWindow($h, 9); [void][AgN]::SetForegroundWindow($h); Start-Sleep -Milliseconds 120
    foreach ($c in $codes) { [AgN]::keybd_event([byte]$c, 0, 0, [IntPtr]::Zero) }
    [array]::Reverse($codes); foreach ($c in $codes) { [AgN]::keybd_event([byte]$c, 0, 2, [IntPtr]::Zero) }
    Start-Sleep -Milliseconds 120; if ($before -ne $h) { [void][AgN]::SetForegroundWindow($before) }
    @{ keys = @($req.keys); window = [int64]$h }
  }
  'select' {
    $t = Target; $ec = Pattern $t.el ([System.Windows.Automation.ExpandCollapsePattern]::Pattern); if ($ec) { $ec.Expand(); Start-Sleep -Milliseconds 150 }
    $cond = New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, "$($req.option)")
    $item = $t.el.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
    if (-not $item) { Fail 'target.stale' "no option named '$($req.option)'" }
    $si = Pattern $item ([System.Windows.Automation.SelectionItemPattern]::Pattern); if ($si) { $si.Select() } else { [void](Click $item) }
    if ($ec) { try { $ec.Collapse() } catch {} }
    @{ selected = "$($req.option)"; value = (Value $t.el) }
  }
  'expand' { $t = Target; $p = Pattern $t.el ([System.Windows.Automation.ExpandCollapsePattern]::Pattern); if (-not $p) { Fail 'target.blocked' 'the element does not expand' }; $p.Expand(); @{ path = $t.path; state = "$($p.Current.ExpandCollapseState)" } }
  'collapse' { $t = Target; $p = Pattern $t.el ([System.Windows.Automation.ExpandCollapsePattern]::Pattern); if (-not $p) { Fail 'target.blocked' 'the element does not collapse' }; $p.Collapse(); @{ path = $t.path; state = "$($p.Current.ExpandCollapseState)" } }
  'scroll' {
    $t = Target
    $si = Pattern $t.el ([System.Windows.Automation.ScrollItemPattern]::Pattern)
    if ($req.intoView -and $si) { $si.ScrollIntoView(); @{ path = $t.path; scrolled = 'into view' } } else {
      $sp = Pattern $t.el ([System.Windows.Automation.ScrollPattern]::Pattern); if (-not $sp) { Fail 'target.blocked' 'the element does not scroll' }
      if ($null -ne $req.percent) { $sp.SetScrollPercent(-1, [double]$req.percent) } else {
        $amount = if ("$($req.direction)" -eq 'up') { [System.Windows.Automation.ScrollAmount]::LargeDecrement } else { [System.Windows.Automation.ScrollAmount]::LargeIncrement }
        $sp.ScrollVertical($amount) }
      @{ path = $t.path; verticalPercent = $sp.Current.VerticalScrollPercent }
    }
  }
  'drag' {
    $t = Target; $from = Center $t.el
    $req.path = $req.toPath; $req.name = $req.toName; $req.automationId = $null; $req.type = $null
    $to = Center (Target).el; $before = New-Object AgN+POINT; [void][AgN]::GetCursorPos([ref]$before)
    [void][AgN]::SetCursorPos($from.x, $from.y); [AgN]::mouse_event(2, 0, 0, 0, [IntPtr]::Zero)
    foreach ($s in 1..10) { [void][AgN]::SetCursorPos($from.x + ($to.x - $from.x) * $s / 10, $from.y + ($to.y - $from.y) * $s / 10); Start-Sleep -Milliseconds 15 }
    [AgN]::mouse_event(4, 0, 0, 0, [IntPtr]::Zero); [void][AgN]::SetCursorPos($before.X, $before.Y)
    @{ from = $from; to = $to }
  }
  'clipboard' { if ($null -ne $req.text) { [System.Windows.Forms.Clipboard]::SetText("$($req.text)"); @{ written = "$($req.text)".Length } } else { @{ text = [System.Windows.Forms.Clipboard]::GetText() } } }
  'dialogs' { @{ dialogs = @(Dialogs) } }
  'dialog' {
    $h = if ($req.hwnd) { [IntPtr][int64]$req.hwnd } else { $d = Dialogs | Select-Object -First 1; if (-not $d) { Fail 'target.stale' 'no dialog is open' }; [IntPtr][int64]$d.hwnd }
    $root = $A::FromHandle($h)
    if ($req.fileName) {
      $edit = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::AutomationIdProperty, '1148')))
      if (-not $edit) { $edit = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit))) }
      if (-not $edit) { Fail 'target.stale' 'the dialog has no file name field' }
      (Pattern $edit ([System.Windows.Automation.ValuePattern]::Pattern)).SetValue("$($req.fileName)")
    }
    $pressed = $null
    if ($req.button) {
      $cond = New-Object System.Windows.Automation.AndCondition((New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)), (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, "$($req.button)")))
      $b = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $cond)
      if (-not $b) { Fail 'target.stale' "the dialog has no button '$($req.button)'" }
      (Pattern $b ([System.Windows.Automation.InvokePattern]::Pattern)).Invoke(); $pressed = "$($req.button)"
    }
    $buttons = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button))) | ForEach-Object { $_.Current.Name } | Where-Object { $_ })
    @{ dialog = [int64]$h; title = (Text $h $false); pressed = $pressed; buttons = $buttons }
  }
  'accessibility' {
    $sr = 0; [void][AgN]::SystemParametersInfo(0x0046, 0, [ref]$sr, 0)
    $hc = New-Object AgN+HIGHCONTRAST; $hc.cbSize = [System.Runtime.InteropServices.Marshal]::SizeOf($hc); [void][AgN]::SystemParametersInfo(0x0042, $hc.cbSize, [ref]$hc, 0)
    $anim = 1; [void][AgN]::SystemParametersInfo(0x1042, 0, [ref]$anim, 0)
    $scale = try { (Get-ItemProperty 'HKCU:\Software\Microsoft\Accessibility' -Name TextScaleFactor -ErrorAction Stop).TextScaleFactor } catch { 100 }
    @{ screenReader = ($sr -ne 0); highContrast = (($hc.dwFlags -band 1) -ne 0); animations = ($anim -ne 0); textScalePercent = $scale }
  }
  'tray' {
    $tray = $A::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children, (New-Object System.Windows.Automation.PropertyCondition($A::ClassNameProperty, 'Shell_TrayWnd')))
    if (-not $tray) { Fail 'target.stale' 'no taskbar' }
    $buttons = $tray.FindAll([System.Windows.Automation.TreeScope]::Descendants, (New-Object System.Windows.Automation.PropertyCondition($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)))
    $icons = @($buttons | Where-Object { $_.Current.Name } | ForEach-Object { @{ name = $_.Current.Name; automationId = $_.Current.AutomationId } })
    if ($req.name) {
      $b = $buttons | Where-Object { $_.Current.Name -like "*$($req.name)*" } | Select-Object -First 1
      if (-not $b) { Fail 'target.stale' "no tray icon named '$($req.name)'" }
      [void](Click $b); @{ clicked = $b.Current.Name; icons = $icons }
    } else { @{ icons = $icons } }
  }
  'toast' {
    [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
    [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom, ContentType = WindowsRuntime]
    $esc = { param($s) [System.Security.SecurityElement]::Escape("$s") }
    $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
    $xml.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$(& $esc $req.title)</text><text>$(& $esc $req.text)</text></binding></visual></toast>")
    $app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show((New-Object Windows.UI.Notifications.ToastNotification $xml))
    @{ shown = $true; title = "$($req.title)" }
  }
  'balloon' {
    $icon = New-Object System.Windows.Forms.NotifyIcon; $icon.Icon = [System.Drawing.SystemIcons]::Information; $icon.Visible = $true
    $icon.ShowBalloonTip(5000, "$($req.title)", "$($req.text)", 'Info'); Start-Sleep -Milliseconds 800; $icon.Dispose()
    @{ shown = $true; title = "$($req.title)" }
  }
  'ocr' {
    [void][Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
    [void][Windows.Graphics.Imaging.BitmapDecoder, Windows.Foundation, ContentType = WindowsRuntime]
    [void][Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
    function Await($op, $type) { $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result }
    $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync("$($req.path)")) ([Windows.Storage.StorageFile])
    $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
    $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
    if (-not $engine) { Fail 'capability.unsupported' 'no Windows OCR language is installed' }
    $result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
    $lines = @($result.Lines | ForEach-Object {
      $words = @($_.Words | ForEach-Object { @{ text = $_.Text; x = [int]$_.BoundingRect.X; y = [int]$_.BoundingRect.Y; width = [int]$_.BoundingRect.Width; height = [int]$_.BoundingRect.Height } })
      @{ text = $_.Text; words = $words } })
    @{ text = $result.Text; lines = $lines; language = $engine.RecognizerLanguage.LanguageTag }
  }
  'spellcheck' {
    # Windows' own spell checker, through a WPF text box that is never shown.
    Add-Type -AssemblyName PresentationFramework
    $tb = New-Object System.Windows.Controls.TextBox
    $tb.Language = [System.Windows.Markup.XmlLanguage]::GetLanguage($(if ($req.language) { "$($req.language)" } else { 'en-US' }))
    $tb.SpellCheck.IsEnabled = $true
    $tb.Text = "$($req.text)"
    $tb.Measure([System.Windows.Size]::new(4000, 4000)); $tb.Arrange([System.Windows.Rect]::new(0, 0, 4000, 4000)); $tb.UpdateLayout()
    $forward = [System.Windows.Documents.LogicalDirection]::Forward
    $errors = @(); $i = $tb.GetNextSpellingErrorCharacterIndex(0, $forward)
    while ($i -ge 0 -and $errors.Count -lt 200) {
      $len = $tb.GetSpellingErrorLength($i)
      $errors += @{ word = $tb.Text.Substring($i, $len); index = $i; suggestions = @($tb.GetSpellingError($i).Suggestions | Select-Object -First 5) }
      $i = $tb.GetNextSpellingErrorCharacterIndex($i + [Math]::Max($len, 1), $forward)
    }
    @{ errors = $errors; count = $errors.Count; language = "$($tb.Language)" }
  }
  'diff' {
    # Changed regions between two pictures of the same size, on a grid.
    Add-Type -ReferencedAssemblies System.Drawing @'
using System; using System.Drawing; using System.Drawing.Imaging; using System.Collections.Generic; using System.Runtime.InteropServices;
public static class AgDiff {
  static byte[] Bytes(Bitmap b, out int stride) {
    var d = b.LockBits(new Rectangle(0, 0, b.Width, b.Height), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    stride = d.Stride; var a = new byte[d.Stride * b.Height]; Marshal.Copy(d.Scan0, a, 0, a.Length); b.UnlockBits(d); return a;
  }
  public static object[] Run(string p1, string p2, int cell, int tolerance) {
    using (var a = new Bitmap(p1)) using (var b = new Bitmap(p2)) {
      if (a.Width != b.Width || a.Height != b.Height) return null;
      int s1, s2; var x1 = Bytes(a, out s1); var x2 = Bytes(b, out s2);
      int cols = (a.Width + cell - 1) / cell, rows = (a.Height + cell - 1) / cell; var hit = new bool[cols, rows]; long changed = 0;
      for (int y = 0; y < a.Height; y++) for (int x = 0; x < a.Width; x++) {
        int i = y * s1 + x * 4, j = y * s2 + x * 4;
        if (Math.Abs(x1[i] - x2[j]) + Math.Abs(x1[i + 1] - x2[j + 1]) + Math.Abs(x1[i + 2] - x2[j + 2]) > tolerance) { hit[x / cell, y / cell] = true; changed++; }
      }
      // Join neighbouring changed cells into boxes.
      var seen = new bool[cols, rows]; var boxes = new List<int[]>();
      for (int cy = 0; cy < rows; cy++) for (int cx = 0; cx < cols; cx++) {
        if (!hit[cx, cy] || seen[cx, cy]) continue;
        int minx = cx, maxx = cx, miny = cy, maxy = cy; var q = new Queue<int[]>(); q.Enqueue(new[] { cx, cy }); seen[cx, cy] = true;
        while (q.Count > 0) { var c = q.Dequeue(); minx = Math.Min(minx, c[0]); maxx = Math.Max(maxx, c[0]); miny = Math.Min(miny, c[1]); maxy = Math.Max(maxy, c[1]);
          foreach (var n in new[] { new[] { c[0] + 1, c[1] }, new[] { c[0] - 1, c[1] }, new[] { c[0], c[1] + 1 }, new[] { c[0], c[1] - 1 } })
            if (n[0] >= 0 && n[1] >= 0 && n[0] < cols && n[1] < rows && hit[n[0], n[1]] && !seen[n[0], n[1]]) { seen[n[0], n[1]] = true; q.Enqueue(n); } }
        boxes.Add(new[] { minx * cell, miny * cell, Math.Min(a.Width, (maxx + 1) * cell) - minx * cell, Math.Min(a.Height, (maxy + 1) * cell) - miny * cell });
      }
      return new object[] { changed, (long)a.Width * a.Height, boxes.ToArray(), a.Width, a.Height };
    }
  }
}
'@
    $r = [AgDiff]::Run("$($req.before)", "$($req.after)", [int]$(if ($req.cell) { $req.cell } else { 16 }), [int]$(if ($req.tolerance) { $req.tolerance } else { 24 }))
    if (-not $r) { Fail 'invalid.params' 'the two pictures differ in size' }
    $regions = @($r[2] | Select-Object -First 50 | ForEach-Object { @{ x = $_[0]; y = $_[1]; width = $_[2]; height = $_[3] } })
    @{ changedPixels = $r[0]; pixels = $r[1]; changed = [Math]::Round(100.0 * $r[0] / [Math]::Max(1, $r[1]), 3); regions = $regions; width = $r[3]; height = $r[4] }
  }
  default { Fail 'invalid.params' "unknown native operation $($req.op)" }
}
$out | ConvertTo-Json -Depth 12 -Compress
