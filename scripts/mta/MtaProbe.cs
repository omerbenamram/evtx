// Ground-truth probe for MTA fixtures: asks wevtapi to render every event of
// an archived .evtx from its LocaleMetaData\*.MTA (EvtOpenPublisherMetadata
// with LogFilePath set, which reads only the archive even when the publisher
// is installed). Writes one JSON object per event.
//
// Compiled by generate-mta-samples.ps1 through Add-Type in Windows
// PowerShell 5.1, so it sticks to C# 5.
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Xml;

public static class MtaProbe
{
    const int EvtQueryFilePath = 0x2;
    const int EvtQueryForwardDirection = 0x100;
    const int EvtRenderEventXml = 1;
    const int ERROR_INSUFFICIENT_BUFFER = 122;
    const int ERROR_NO_MORE_ITEMS = 259;

    const int EvtFormatMessageEvent = 1;
    const int EvtFormatMessageLevel = 2;
    const int EvtFormatMessageTask = 3;
    const int EvtFormatMessageOpcode = 4;
    const int EvtFormatMessageKeyword = 5;

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtQuery(IntPtr session, string path, string query, int flags);

    [DllImport("wevtapi.dll", SetLastError = true)]
    static extern bool EvtNext(IntPtr resultSet, int eventArraySize, [Out] IntPtr[] eventArray, int timeout, int flags, out int returned);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr EvtOpenPublisherMetadata(IntPtr session, string publisherId, string logFilePath, int locale, int flags);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool EvtFormatMessage(IntPtr publisherMetadata, IntPtr eventHandle, uint messageId, int valueCount, IntPtr values, int flags, int bufferSize, [Out] char[] buffer, out int bufferUsed);

    [DllImport("wevtapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool EvtRender(IntPtr context, IntPtr fragment, int flags, int bufferSize, [Out] char[] buffer, out int bufferUsed, out int propertyCount);

    [DllImport("wevtapi.dll")]
    static extern bool EvtClose(IntPtr handle);

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
        if (err != ERROR_INSUFFICIENT_BUFFER)
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

    static string RenderXml(IntPtr evt)
    {
        int used, props;
        EvtRender(IntPtr.Zero, evt, EvtRenderEventXml, 0, null, out used, out props);
        int err = Marshal.GetLastWin32Error();
        if (err != ERROR_INSUFFICIENT_BUFFER)
        {
            throw new System.ComponentModel.Win32Exception(err);
        }
        char[] buffer = new char[used / 2 + 1];
        if (!EvtRender(IntPtr.Zero, evt, EvtRenderEventXml, buffer.Length * 2, buffer, out used, out props))
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        return new string(buffer, 0, used / 2).TrimEnd('\0');
    }

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
                    if (c < 0x20)
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

    static void Field(StringBuilder sb, string name, Formatted f)
    {
        sb.Append(",");
        Json(sb, name);
        sb.Append(":");
        // Keep the raw text, including embedded and trailing NULs, so the
        // fixture shows exactly what wevtapi returned.
        Json(sb, f.Text);
        sb.Append(",");
        Json(sb, name + "_error");
        sb.Append(":").Append(f.Error);
    }

    static string Child(XmlElement system, XmlNamespaceManager ns, string name)
    {
        XmlNode n = system.SelectSingleNode("e:" + name, ns);
        return n == null ? null : n.InnerText;
    }

    public static int Probe(string evtxPath, int lcid, string outPath)
    {
        IntPtr query = EvtQuery(IntPtr.Zero, evtxPath, "*", EvtQueryFilePath | EvtQueryForwardDirection);
        if (query == IntPtr.Zero)
        {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }

        Dictionary<string, IntPtr> metadata = new Dictionary<string, IntPtr>();
        Dictionary<string, int> metadataErrors = new Dictionary<string, int>();
        IntPtr[] events = new IntPtr[64];
        int returned;
        int index = 0;
        UTF8Encoding utf8 = new UTF8Encoding(false);
        using (StreamWriter writer = new StreamWriter(outPath, false, utf8))
        {
            while (EvtNext(query, events.Length, events, -1, 0, out returned))
            {
                for (int i = 0; i < returned; i++)
                {
                    IntPtr evt = events[i];
                    try
                    {
                        string xml = RenderXml(evt);
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
                        sb.Append(",\"provider\":"); Json(sb, providerName);
                        sb.Append(",\"event_record_id\":"); Json(sb, Child(system, ns, "EventRecordID"));
                        XmlElement eventId = (XmlElement)system.SelectSingleNode("e:EventID", ns);
                        sb.Append(",\"event_id\":"); Json(sb, eventId.InnerText);
                        sb.Append(",\"qualifiers\":"); Json(sb, eventId.HasAttribute("Qualifiers") ? eventId.GetAttribute("Qualifiers") : null);
                        sb.Append(",\"level\":"); Json(sb, Child(system, ns, "Level"));
                        sb.Append(",\"task\":"); Json(sb, Child(system, ns, "Task"));
                        sb.Append(",\"opcode\":"); Json(sb, Child(system, ns, "Opcode"));
                        sb.Append(",\"keywords\":"); Json(sb, Child(system, ns, "Keywords"));
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
                        EvtClose(evt);
                    }
                    index++;
                }
            }
            int nextError = Marshal.GetLastWin32Error();
            if (nextError != ERROR_NO_MORE_ITEMS)
            {
                throw new System.ComponentModel.Win32Exception(nextError);
            }
        }
        foreach (IntPtr handle in metadata.Values)
        {
            if (handle != IntPtr.Zero)
            {
                EvtClose(handle);
            }
        }
        EvtClose(query);
        return index;
    }
}
