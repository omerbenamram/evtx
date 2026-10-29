// Ground-truth probe for MTA fixtures: asks wevtapi to render every event of
// an archived .evtx from its LocaleMetaData\*.MTA (EvtOpenPublisherMetadata
// with LogFilePath set, which reads only the archive even when the publisher
// is installed). Writes one JSON object per event.
//
// Compiled with ../windows/Wevtapi.cs by generate-mta-samples.ps1 through
// Add-Type in Windows PowerShell 5.1, so it sticks to C# 5.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Xml;

public static class MtaProbe
{
    const int EvtFormatMessageEvent = 1;
    const int EvtFormatMessageLevel = 2;
    const int EvtFormatMessageTask = 3;
    const int EvtFormatMessageOpcode = 4;
    const int EvtFormatMessageKeyword = 5;

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtOpenPublisherMetadata(IntPtr session, string publisherId, string logFilePath, int locale, int flags);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool EvtFormatMessage(IntPtr publisherMetadata, IntPtr eventHandle, uint messageId, int valueCount, IntPtr values, int flags, int bufferSize, [Out] char[] buffer, out int bufferUsed);

    class Formatted
    {
        public string Text;
        public int Error;
    }

    static Formatted Format(IntPtr metadata, IntPtr evt, int flags)
    {
        Formatted result = new Formatted();
        int used;
        if (EvtFormatMessage(metadata, evt, 0, 0, IntPtr.Zero, flags, 0, null, out used))
        {
            result.Text = "";
            return result;
        }
        int err = Marshal.GetLastWin32Error();
        if (err != Wevtapi.ERROR_INSUFFICIENT_BUFFER)
        {
            result.Error = err;
            return result;
        }
        char[] buffer = new char[used];
        if (!EvtFormatMessage(metadata, evt, 0, 0, IntPtr.Zero, flags, buffer.Length, buffer, out used))
        {
            // Some errors (e.g. unresolved inserts) still fill the buffer.
            result.Error = Marshal.GetLastWin32Error();
            if (used > 0 && used <= buffer.Length)
            {
                result.Text = new string(buffer, 0, used);
            }
            return result;
        }
        result.Text = new string(buffer, 0, used);
        return result;
    }

    static void Field(StringBuilder sb, string name, Formatted f)
    {
        sb.Append(",");
        Wevtapi.Json(sb, name);
        sb.Append(":");
        // Keep the raw text, including embedded and trailing NULs, so the
        // fixture shows exactly what wevtapi returned.
        Wevtapi.Json(sb, f.Text);
        sb.Append(",");
        Wevtapi.Json(sb, name + "_error");
        sb.Append(":").Append(f.Error);
    }

    static string Child(XmlElement system, XmlNamespaceManager ns, string name)
    {
        XmlNode n = system.SelectSingleNode("e:" + name, ns);
        return n == null ? null : n.InnerText;
    }

    public static int Probe(string evtxPath, int lcid, string outPath)
    {
        IntPtr query = Wevtapi.EvtQuery(IntPtr.Zero, evtxPath, "*", Wevtapi.EvtQueryFilePath | Wevtapi.EvtQueryForwardDirection);
        if (query == IntPtr.Zero)
        {
            throw new Win32Exception(Marshal.GetLastWin32Error());
        }

        Dictionary<string, IntPtr> metadata = new Dictionary<string, IntPtr>();
        Dictionary<string, int> metadataErrors = new Dictionary<string, int>();
        IntPtr[] events = new IntPtr[64];
        int returned;
        int index = 0;
        UTF8Encoding utf8 = new UTF8Encoding(false);
        using (StreamWriter writer = new StreamWriter(outPath, false, utf8))
        {
            while (Wevtapi.EvtNext(query, events.Length, events, -1, 0, out returned))
            {
                for (int i = 0; i < returned; i++)
                {
                    IntPtr evt = events[i];
                    try
                    {
                        string xml = Wevtapi.Render(evt, Wevtapi.EvtRenderEventXml);
                        XmlDocument doc = new XmlDocument();
                        doc.LoadXml(xml);
                        XmlNamespaceManager ns = new XmlNamespaceManager(doc.NameTable);
                        ns.AddNamespace("e", "http://schemas.microsoft.com/win/2004/08/events/event");
                        XmlElement system = (XmlElement)doc.SelectSingleNode("/e:Event/e:System", ns);
                        XmlElement provider = (XmlElement)system.SelectSingleNode("e:Provider", ns);
                        string providerName = provider.GetAttribute("Name");

                        IntPtr pm;
                        if (!metadata.TryGetValue(providerName, out pm))
                        {
                            pm = EvtOpenPublisherMetadata(IntPtr.Zero, providerName, evtxPath, lcid, 0);
                            metadata[providerName] = pm;
                            metadataErrors[providerName] = pm == IntPtr.Zero ? Marshal.GetLastWin32Error() : 0;
                        }

                        StringBuilder sb = new StringBuilder();
                        sb.Append("{\"index\":").Append(index);
                        sb.Append(",\"provider\":"); Wevtapi.Json(sb, providerName);
                        sb.Append(",\"event_record_id\":"); Wevtapi.Json(sb, Child(system, ns, "EventRecordID"));
                        XmlElement eventId = (XmlElement)system.SelectSingleNode("e:EventID", ns);
                        sb.Append(",\"event_id\":"); Wevtapi.Json(sb, eventId.InnerText);
                        sb.Append(",\"qualifiers\":"); Wevtapi.Json(sb, eventId.HasAttribute("Qualifiers") ? eventId.GetAttribute("Qualifiers") : null);
                        sb.Append(",\"level\":"); Wevtapi.Json(sb, Child(system, ns, "Level"));
                        sb.Append(",\"task\":"); Wevtapi.Json(sb, Child(system, ns, "Task"));
                        sb.Append(",\"opcode\":"); Wevtapi.Json(sb, Child(system, ns, "Opcode"));
                        sb.Append(",\"keywords\":"); Wevtapi.Json(sb, Child(system, ns, "Keywords"));
                        sb.Append(",\"metadata_error\":").Append(metadataErrors[providerName]);
                        if (pm != IntPtr.Zero)
                        {
                            Field(sb, "message", Format(pm, evt, EvtFormatMessageEvent));
                            Field(sb, "level_name", Format(pm, evt, EvtFormatMessageLevel));
                            Field(sb, "task_name", Format(pm, evt, EvtFormatMessageTask));
                            Field(sb, "opcode_name", Format(pm, evt, EvtFormatMessageOpcode));
                            Field(sb, "keyword_names", Format(pm, evt, EvtFormatMessageKeyword));
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
            }
            int nextError = Marshal.GetLastWin32Error();
            if (nextError != Wevtapi.ERROR_NO_MORE_ITEMS)
            {
                throw new Win32Exception(nextError);
            }
        }
        foreach (IntPtr handle in metadata.Values)
        {
            if (handle != IntPtr.Zero)
            {
                Wevtapi.EvtClose(handle);
            }
        }
        Wevtapi.EvtClose(query);
        return index;
    }
}
