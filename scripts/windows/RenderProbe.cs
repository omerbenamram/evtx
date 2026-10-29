// Ground-truth probe for XML rendering: asks wevtapi for its own XML of every
// event in an .evtx (EvtQuery with EvtQueryFilePath, EvtRender with
// EvtRenderEventXml) and writes one JSON object per event, in the order
// EvtNext returns them.
//
// Compiled by render-samples.ps1 through Add-Type in Windows PowerShell 5.1,
// so it sticks to C# 5.
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

public static class RenderProbe
{
    const int EvtQueryFilePath = 0x2;
    const int EvtQueryForwardDirection = 0x100;
    const int EvtRenderEventXml = 1;
    const int EvtRenderBookmark = 2;
    const int EvtSeekRelativeToFirst = 1;
    const int EvtSeekStrict = 0x10000;
    const int ERROR_INSUFFICIENT_BUFFER = 122;
    const int ERROR_NO_MORE_ITEMS = 259;
    // Give up on a file after this many EvtNext failures in a row.
    const int MaxConsecutiveErrors = 64;

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtQuery(IntPtr session, string path, string query, int flags);

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtNext(IntPtr resultSet, int eventArraySize, [Out] IntPtr[] eventArray, int timeout, int flags, out int returned);

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtSeek(IntPtr resultSet, long position, IntPtr bookmark, int timeout, int flags);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtCreateBookmark(string bookmarkXml);

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtUpdateBookmark(IntPtr bookmark, IntPtr evt);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool EvtRender(IntPtr context, IntPtr fragment, int flags, int bufferSize, [Out] char[] buffer, out int bufferUsed, out int propertyCount);

    [DllImport("wevtapi.dll")]
    static extern bool EvtClose(IntPtr handle);

    // Reused across calls; grows when wevtapi asks for more.
    static char[] buffer = new char[16384];

    static string Render(IntPtr handle, int flags)
    {
        int used, props;
        if (!EvtRender(IntPtr.Zero, handle, flags, buffer.Length * 2, buffer, out used, out props))
        {
            int err = Marshal.GetLastWin32Error();
            if (err != ERROR_INSUFFICIENT_BUFFER)
            {
                throw new System.ComponentModel.Win32Exception(err);
            }
            buffer = new char[used / 2 + 1];
            if (!EvtRender(IntPtr.Zero, handle, flags, buffer.Length * 2, buffer, out used, out props))
            {
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            }
        }
        return new string(buffer, 0, used / 2).TrimEnd('\0');
    }

    // JSON string with every control character and every surrogate escaped,
    // so lone surrogates survive the UTF-8 file as \uXXXX.
    static void Json(StringBuilder sb, string s)
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
                    if (c < 0x20 || (c >= 0xD800 && c <= 0xDFFF))
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

    static readonly Regex RecordIdAttribute = new Regex("RecordId='(\\d+)'");

    // The record number of the bookmark wevtapi makes for the event (the record
    // header's number, which is not always the EventRecordID in the XML).
    static string BookmarkRecordId(IntPtr bookmark, IntPtr evt)
    {
        if (!EvtUpdateBookmark(bookmark, evt))
        {
            return null;
        }
        try
        {
            Match m = RecordIdAttribute.Match(Render(bookmark, EvtRenderBookmark));
            return m.Success ? m.Groups[1].Value : null;
        }
        catch (System.ComponentModel.Win32Exception)
        {
            return null;
        }
    }

    public static int Probe(string evtxPath, string outPath)
    {
        UTF8Encoding utf8 = new UTF8Encoding(false);
        using (StreamWriter writer = new StreamWriter(outPath, false, utf8))
        {
            writer.NewLine = "\n";
            IntPtr query = EvtQuery(IntPtr.Zero, evtxPath, "*", EvtQueryFilePath | EvtQueryForwardDirection);
            if (query == IntPtr.Zero)
            {
                writer.WriteLine("{\"query_error\":" + Marshal.GetLastWin32Error() + "}");
                return 0;
            }
            IntPtr bookmark = EvtCreateBookmark(null);
            IntPtr[] events = new IntPtr[1];
            int returned;
            int index = 0;
            int consecutiveErrors = 0;
            while (true)
            {
                if (!EvtNext(query, 1, events, -1, 0, out returned))
                {
                    int err = Marshal.GetLastWin32Error();
                    if (err == ERROR_NO_MORE_ITEMS)
                    {
                        break;
                    }
                    StringBuilder e = new StringBuilder();
                    e.Append("{\"index\":").Append(index).Append(",\"next_error\":").Append(err);
                    // Step over the event wevtapi could not read. Strict, so seeking past the
                    // last event fails instead of landing on it again.
                    bool seeked = EvtSeek(query, index + 1, IntPtr.Zero, 0, EvtSeekRelativeToFirst | EvtSeekStrict);
                    e.Append(",\"seek_error\":").Append(seeked ? 0 : Marshal.GetLastWin32Error()).Append("}");
                    writer.WriteLine(e.ToString());
                    index++;
                    if (!seeked || ++consecutiveErrors >= MaxConsecutiveErrors)
                    {
                        break;
                    }
                    continue;
                }
                consecutiveErrors = 0;
                IntPtr evt = events[0];
                try
                {
                    StringBuilder sb = new StringBuilder();
                    sb.Append("{\"index\":").Append(index);
                    sb.Append(",\"record_id\":").Append(BookmarkRecordId(bookmark, evt) ?? "null");
                    try
                    {
                        string xml = Render(evt, EvtRenderEventXml);
                        sb.Append(",\"xml\":");
                        Json(sb, xml);
                    }
                    catch (System.ComponentModel.Win32Exception ex)
                    {
                        sb.Append(",\"render_error\":").Append(ex.NativeErrorCode);
                    }
                    sb.Append("}");
                    writer.WriteLine(sb.ToString());
                }
                finally
                {
                    EvtClose(evt);
                }
                index++;
            }
            EvtClose(bookmark);
            EvtClose(query);
            return index;
        }
    }
}
