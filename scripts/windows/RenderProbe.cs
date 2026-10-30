// Ground-truth probe for XML rendering: asks wevtapi for its own XML of every
// event in an .evtx (EvtQuery with EvtQueryFilePath, EvtRender with
// EvtRenderEventXml) and writes one JSON object per event, in the order
// EvtNext returns them.
//
// Compiled with Wevtapi.cs by render-samples.ps1 through Add-Type in Windows
// PowerShell 5.1, so it sticks to C# 5.
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

public static class RenderProbe
{
    const int EvtSeekRelativeToFirst = 1;
    const int EvtSeekStrict = 0x10000;

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtSeek(IntPtr resultSet, long position, IntPtr bookmark, int timeout, int flags);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtCreateBookmark(string bookmarkXml);

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtUpdateBookmark(IntPtr bookmark, IntPtr evt);

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
            Match m = RecordIdAttribute.Match(Wevtapi.Render(bookmark, Wevtapi.EvtRenderBookmark));
            return m.Success ? m.Groups[1].Value : null;
        }
        catch (Win32Exception)
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
            IntPtr query = Wevtapi.EvtQuery(IntPtr.Zero, evtxPath, "*", Wevtapi.EvtQueryFilePath | Wevtapi.EvtQueryForwardDirection);
            if (query == IntPtr.Zero)
            {
                writer.WriteLine("{\"query_error\":" + Marshal.GetLastWin32Error() + "}");
                return 0;
            }
            IntPtr bookmark = EvtCreateBookmark(null);
            IntPtr[] events = new IntPtr[1];
            int returned;
            int index = 0;
            while (true)
            {
                if (!Wevtapi.EvtNext(query, 1, events, -1, 0, out returned))
                {
                    int err = Marshal.GetLastWin32Error();
                    if (err == Wevtapi.ERROR_NO_MORE_ITEMS)
                    {
                        break;
                    }
                    // Step over the event wevtapi could not read. Strict, so seeking past the
                    // last event fails, which ends the loop, instead of landing on it again.
                    bool seeked = EvtSeek(query, index + 1, IntPtr.Zero, 0, EvtSeekRelativeToFirst | EvtSeekStrict);
                    int seekError = seeked ? 0 : Marshal.GetLastWin32Error();
                    writer.WriteLine("{\"index\":" + index + ",\"next_error\":" + err + ",\"seek_error\":" + seekError + "}");
                    index++;
                    if (!seeked)
                    {
                        break;
                    }
                    continue;
                }
                IntPtr evt = events[0];
                try
                {
                    StringBuilder sb = new StringBuilder();
                    sb.Append("{\"index\":").Append(index);
                    sb.Append(",\"record_id\":").Append(BookmarkRecordId(bookmark, evt) ?? "null");
                    try
                    {
                        string xml = Wevtapi.Render(evt, Wevtapi.EvtRenderEventXml);
                        sb.Append(",\"xml\":");
                        Wevtapi.Json(sb, xml);
                    }
                    catch (Win32Exception ex)
                    {
                        sb.Append(",\"render_error\":").Append(ex.NativeErrorCode);
                    }
                    sb.Append("}");
                    writer.WriteLine(sb.ToString());
                }
                finally
                {
                    Wevtapi.EvtClose(evt);
                }
                index++;
            }
            Wevtapi.EvtClose(bookmark);
            Wevtapi.EvtClose(query);
            return index;
        }
    }
}
