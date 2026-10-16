# Stamps the AgentGo icon and version information onto an executable.
# The sandboxed bundle's agentgo.exe is CEF's generic bootstrap.exe, which
# carries neither, so Explorer, the taskbar and Task Manager would show a
# blank program called "bootstrap". Uses the Win32 UpdateResource API; no
# external tool is needed.
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Icon = (Join-Path $PSScriptRoot "..\crates\agentgo-app\assets\agentgo.ico"),
    [string]$Version = "0.1.0"
)
$ErrorActionPreference = "Stop"

Add-Type -TypeDefinition @"
using System;
using System.IO;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class ExeStamp {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr BeginUpdateResource(string file, bool deleteExisting);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool UpdateResource(IntPtr h, IntPtr type, IntPtr name, ushort lang, byte[] data, uint size);
    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool EndUpdateResource(IntPtr h, bool discard);

    static void Pad4(List<byte> o) { while (o.Count % 4 != 0) o.Add(0); }

    static byte[] Node(string key, bool text, byte[] value, List<byte[]> children) {
        var o = new List<byte>(new byte[6]);
        o.AddRange(Encoding.Unicode.GetBytes(key)); o.Add(0); o.Add(0);
        Pad4(o);
        o.AddRange(value);
        foreach (var c in children) { Pad4(o); o.AddRange(c); }
        int valueLen = text ? value.Length / 2 : value.Length;
        var a = o.ToArray();
        BitConverter.GetBytes((ushort)a.Length).CopyTo(a, 0);
        BitConverter.GetBytes((ushort)valueLen).CopyTo(a, 2);
        BitConverter.GetBytes((ushort)(text ? 1 : 0)).CopyTo(a, 4);
        return a;
    }

    static byte[] Str(string key, string value) {
        var v = new List<byte>(Encoding.Unicode.GetBytes(value)); v.Add(0); v.Add(0);
        return Node(key, true, v.ToArray(), new List<byte[]>());
    }

    static byte[] VersionResource(string version) {
        var p = version.Split('.');
        uint major = uint.Parse(p[0]), minor = uint.Parse(p[1]), patch = uint.Parse(p[2]);
        uint high = (major << 16) | minor, low = patch << 16;
        uint[] fields = { 0xFEEF04BD, 0x00010000, high, low, high, low, 0x3F, 0, 0x00040004, 1, 0, 0, 0 };
        var fixedInfo = new List<byte>();
        foreach (var f in fields) fixedInfo.AddRange(BitConverter.GetBytes(f));
        var strings = Node("040904B0", true, new byte[0], new List<byte[]> {
            Str("CompanyName", "AgentGo"), Str("FileDescription", "AgentGo"),
            Str("FileVersion", version), Str("InternalName", "agentgo"),
            Str("OriginalFilename", "agentgo.exe"), Str("ProductName", "AgentGo"),
            Str("ProductVersion", version) });
        var translation = Node("Translation", false, new byte[] { 9, 4, 0xB0, 4 }, new List<byte[]>());
        return Node("VS_VERSION_INFO", false, fixedInfo.ToArray(), new List<byte[]> {
            Node("StringFileInfo", true, new byte[0], new List<byte[]> { strings }),
            Node("VarFileInfo", true, new byte[0], new List<byte[]> { translation }) });
    }

    static void Put(IntPtr h, int type, int name, byte[] data) {
        if (!UpdateResource(h, (IntPtr)type, (IntPtr)name, 0, data, (uint)data.Length))
            throw new Exception("UpdateResource failed: " + Marshal.GetLastWin32Error());
    }

    public static void Stamp(string exe, string icon, string version) {
        var ico = File.ReadAllBytes(icon);
        int count = BitConverter.ToUInt16(ico, 4);
        IntPtr h = BeginUpdateResource(exe, false);
        if (h == IntPtr.Zero) throw new Exception("BeginUpdateResource failed: " + Marshal.GetLastWin32Error());
        try {
            var group = new List<byte> { 0, 0, 1, 0 };
            group.AddRange(BitConverter.GetBytes((ushort)count));
            for (int i = 0; i < count; i++) {
                int e = 6 + i * 16;
                int size = BitConverter.ToInt32(ico, e + 8), off = BitConverter.ToInt32(ico, e + 12);
                var img = new byte[size]; Array.Copy(ico, off, img, 0, size);
                Put(h, 3, i + 1, img);
                for (int k = 0; k < 12; k++) group.Add(ico[e + k]);
                group.AddRange(BitConverter.GetBytes((ushort)(i + 1)));
            }
            Put(h, 14, 1, group.ToArray());
            Put(h, 16, 1, VersionResource(version));
        } catch { EndUpdateResource(h, true); throw; }
        if (!EndUpdateResource(h, false)) throw new Exception("EndUpdateResource failed: " + Marshal.GetLastWin32Error());
    }
}
"@

# bootstrap.exe is a console program: double-clicking it opens a terminal window
# full of log lines. Mark it a Windows (GUI) program so no console ever appears.
$bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Exe).Path)
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
$subsystemAt = $pe + 24 + 68
if ([BitConverter]::ToUInt16($bytes, $subsystemAt) -eq 3) {
    $bytes[$subsystemAt] = 2
    $bytes[$subsystemAt + 1] = 0
    [IO.File]::WriteAllBytes((Resolve-Path -LiteralPath $Exe).Path, $bytes)
}

[ExeStamp]::Stamp((Resolve-Path -LiteralPath $Exe).Path, (Resolve-Path -LiteralPath $Icon).Path, $Version)
Write-Host "[stamp] icon and version set on $Exe"
