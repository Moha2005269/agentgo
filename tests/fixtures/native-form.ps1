# A small native window for the native lane tests (WPF, which exposes full
# UI Automation): a text box, a button that writes "pressed" into a label,
# a check box, a combo box, a tree, a long list and a double-click target.
# Closes itself after 120 s so a failed test leaves nothing behind.
param([string]$Title = 'AgentGo Native Fixture', [switch]$Legacy)
if ($Legacy) {
  Add-Type -AssemblyName System.Windows.Forms
  $f = New-Object System.Windows.Forms.Form; $f.Text = "$Title Legacy"; $f.StartPosition = 'Manual'; $f.Location = New-Object System.Drawing.Point(480, 40)
  $l = New-Object System.Windows.Forms.Label; $l.Text = 'double me'; $l.SetBounds(10, 10, 200, 24); $l.Add_DoubleClick({ $l.Text = 'doubled' })
  $b = New-Object System.Windows.Forms.TextBox; $b.SetBounds(10, 40, 200, 24); $b.Add_KeyDown({ param($s, $e) $l.Text = "key $($e.KeyCode)" })
  $f.Controls.AddRange(@($l, $b)); $t = New-Object System.Windows.Forms.Timer; $t.Interval = 120000; $t.Add_Tick({ $f.Close() }); $t.Start()
  # The first show takes Windows' hidden start flag; the dialog then shows.
  $f.Show(); $f.Hide(); [void]$f.ShowDialog(); exit
}
Add-Type -AssemblyName PresentationFramework
[xml]$xaml = @"
<Window xmlns='http://schemas.microsoft.com/winfx/2006/xaml/presentation' Title='$Title' Width='420' Height='420' Left='40' Top='40' ShowActivated='False'>
  <StackPanel Margin='8'>
    <StackPanel Orientation='Horizontal'>
      <TextBox Name='Entry' AutomationProperties.Name='Entry' Width='200'/>
      <Button Name='Press' Content='Press' Width='80' Margin='6,0'/>
      <Label Name='Status' AutomationProperties.AutomationId='Status' Content='idle'/>
    </StackPanel>
    <StackPanel Orientation='Horizontal' Margin='0,6'>
      <CheckBox Name='Agree' Content='Agree' Width='100'/>
      <ComboBox Name='Color' AutomationProperties.Name='Color' Width='120'>
        <ComboBoxItem Content='Red'/><ComboBoxItem Content='Green'/><ComboBoxItem Content='Blue'/>
      </ComboBox>
    </StackPanel>
    <StackPanel Orientation='Horizontal'>
      <TreeView AutomationProperties.Name='Tree' Width='180' Height='120'>
        <TreeViewItem Header='Root'><TreeViewItem Header='Child'/></TreeViewItem>
      </TreeView>
      <ListBox Name='Items' AutomationProperties.Name='Items' Width='180' Height='120' Margin='6,0'/>
    </StackPanel>
    <Border Name='Twice' AutomationProperties.Name='Twice' Background='#eee' Height='28' Margin='0,6'>
      <TextBlock Name='TwiceText' Text='double me'/>
    </Border>
  </StackPanel>
</Window>
"@
$window = [Windows.Markup.XamlReader]::Load((New-Object System.Xml.XmlNodeReader $xaml))
$find = { param($n) $window.FindName($n) }
1..60 | ForEach-Object { [void](& $find 'Items').Items.Add("Item $_") }
(& $find 'Press').Add_Click({ (& $find 'Status').Content = 'pressed' })
(& $find 'Twice').Add_MouseLeftButtonDown({ param($s, $e) if ($e.ClickCount -ge 2) { (& $find 'TwiceText').Text = 'doubled' } })
# A classic Win32 (WinForms) window as well, in its own process (WinForms
# inside a WPF message loop does not take posted messages): these controls
# have no UI Automation patterns and are driven with window messages.
Start-Process powershell -WindowStyle Hidden -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Legacy', '-Title', "`"$Title`""
$timer = New-Object System.Windows.Threading.DispatcherTimer; $timer.Interval = [TimeSpan]::FromSeconds(120)
$timer.Add_Tick({ $window.Close() }); $timer.Start()
# Show, not ShowDialog: a modal window would disable the legacy one.
$window.Add_Closed({ [System.Windows.Threading.Dispatcher]::CurrentDispatcher.InvokeShutdown() })
# The first show takes a hidden start flag (no console run): show twice.
$window.Show(); $window.Hide(); $window.Show()
[System.Windows.Threading.Dispatcher]::Run()
