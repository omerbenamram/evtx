// wevtapi interop shared by RenderProbe.cs and ../mta/MtaProbe.cs. Each script
// compiles it together with its probe (Add-Type -Path), so every probe's
// assembly has its own internal copy.
//
// Compiled through Add-Type in Windows PowerShell 5.1, so it sticks to C# 5.
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

internal static class Wevtapi
{
    public const int EvtQueryFilePath = 0x2;
    public const int EvtQueryForwardDirection = 0x100;
    public const int EvtRenderEventXml = 1;
    public const int EvtRenderBookmark = 2;
    public const int ERROR_INSUFFICIENT_BUFFER = 122;
    public const int ERROR_NO_MORE_ITEMS = 259;

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr EvtQuery(IntPtr session, string path, string query, int flags);

    [DllImport("wevtapi.dll", SetLastError = true)]
    public static extern bool EvtNext(IntPtr resultSet, int eventArraySize, [Out] IntPtr[] eventArray, int timeout, int flags, out int returned);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool EvtRender(IntPtr context, IntPtr fragment, int flags, int bufferSize, [Out] char[] buffer, out int bufferUsed, out int propertyCount);

    [DllImport("wevtapi.dll")]
    public static extern bool EvtClose(IntPtr handle);

    // Reused across calls; grows when wevtapi asks for more.
    static char[] buffer = new char[16384];

    // EvtRender of an event or a bookmark, as a string. Throws Win32Exception.
    public static string Render(IntPtr handle, int flags)
    {
        int used, props;
        if (!EvtRender(IntPtr.Zero, handle, flags, buffer.Length * 2, buffer, out used, out props))
        {
            int err = Marshal.GetLastWin32Error();
            if (err != ERROR_INSUFFICIENT_BUFFER)
            {
                throw new Win32Exception(err);
            }
            buffer = new char[used / 2 + 1];
            if (!EvtRender(IntPtr.Zero, handle, flags, buffer.Length * 2, buffer, out used, out props))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error());
            }
        }
        return new string(buffer, 0, used / 2).TrimEnd('\0');
    }

    // JSON string (or null) with every control character and every surrogate
    // escaped, so lone surrogates survive the UTF-8 file as \uXXXX.
    public static void Json(StringBuilder sb, string s)
    {
        if (s == null)
        {
            sb.Append("null");
            return;
        }
        sb.Append('"');
        foreach (char c in s)
        {
            switch (c)
            {
                case '"': sb.Append("\\\""); break;
                case '\\': sb.Append("\\\\"); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (c < 0x20 || char.IsSurrogate(c))
                    {
                        sb.Append("\\u").Append(((int)c).ToString("x4"));
                    }
                    else
                    {
                        sb.Append(c);
                    }
                    break;
            }
        }
        sb.Append('"');
    }
}
