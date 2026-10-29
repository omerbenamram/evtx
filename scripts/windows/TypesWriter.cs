// Writes the events of TypesTest.man through EventWrite, with every payload
// built byte by byte (see generate-types-sample.ps1).
//
// Compiled through Add-Type in Windows PowerShell 5.1, so it sticks to C# 5.
using System;
using System.Collections;
using System.Collections.Generic;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using System.Threading;

public static class TypesWriter
{
    [StructLayout(LayoutKind.Sequential)]
    struct EVENT_DESCRIPTOR
    {
        public ushort Id;
        public byte Version;
        public byte Channel;
        public byte Level;
        public byte Opcode;
        public ushort Task;
        public ulong Keyword;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct EVENT_DATA_DESCRIPTOR
    {
        public ulong Ptr;
        public uint Size;
        public uint Reserved;
    }

    [DllImport("advapi32.dll")]
    static extern uint EventRegister([In] ref Guid providerId, IntPtr enableCallback, IntPtr callbackContext, out long regHandle);

    // Returns a one-byte BOOLEAN, not a four-byte BOOL.
    [DllImport("advapi32.dll")]
    [return: MarshalAs(UnmanagedType.U1)]
    static extern bool EventEnabled(long regHandle, [In] ref EVENT_DESCRIPTOR descriptor);

    [DllImport("advapi32.dll")]
    static extern uint EventWrite(long regHandle, [In] ref EVENT_DESCRIPTOR descriptor, uint userDataCount, [In] EVENT_DATA_DESCRIPTOR[] userData);

    [DllImport("advapi32.dll")]
    static extern uint EventUnregister(long regHandle);

    // An event payload in ETW's wire format.
    public sealed class P
    {
        readonly MemoryStream ms = new MemoryStream();
        readonly BinaryWriter w;

        public P() { w = new BinaryWriter(ms); }

        public byte[] Bytes() { w.Flush(); return ms.ToArray(); }

        // NUL-terminated UTF-16, unit by unit, so lone surrogates stay as they are.
        public P Str(string s) { Units(s); w.Write((ushort)0); return this; }
        // win:UnicodeString with a length: exactly these units, no terminator.
        public P Units(string s) { foreach (char c in s) w.Write((ushort)c); return this; }
        public P Ansi(byte[] b) { w.Write(b); w.Write((byte)0); return this; }
        public P Ansi(string s) { return Ansi(Encoding.ASCII.GetBytes(s)); }
        public P I8(sbyte v) { w.Write(v); return this; }
        public P U8(byte v) { w.Write(v); return this; }
        public P I16(short v) { w.Write(v); return this; }
        public P U16(ushort v) { w.Write(v); return this; }
        public P I32(int v) { w.Write(v); return this; }
        public P U32(uint v) { w.Write(v); return this; }
        public P I64(long v) { w.Write(v); return this; }
        public P U64(ulong v) { w.Write(v); return this; }
        public P F32(float v) { w.Write(v); return this; }
        public P F64(double v) { w.Write(v); return this; }
        public P Raw(byte[] b) { w.Write(b); return this; }
        public P Guid(string g) { w.Write(new Guid(g).ToByteArray()); return this; }
        public P FileTime(long v) { w.Write(v); return this; }
        public P FileTime(DateTime utc) { w.Write(utc.ToFileTimeUtc()); return this; }
        // year, month, day of week, day, hour, minute, second, milliseconds.
        public P SysTime(int[] fields)
        {
            foreach (int v in fields)
            {
                w.Write((ushort)v);
            }
            return this;
        }
        public P Sid(string sddl)
        {
            SecurityIdentifier sid = new SecurityIdentifier(sddl);
            byte[] b = new byte[sid.BinaryLength];
            sid.GetBinaryForm(b, 0);
            w.Write(b);
            return this;
        }
        // win:Binary preceded by its UInt32 length.
        public P Bin(byte[] b) { w.Write((uint)b.Length); w.Write(b); return this; }
    }

    static byte[] Hex(string hex)
    {
        byte[] b = new byte[hex.Length / 2];
        for (int i = 0; i < b.Length; i++)
        {
            b[i] = Convert.ToByte(hex.Substring(i * 2, 2), 16);
        }
        return b;
    }

    static DateTime Utc(int y, int mo, int d, int h, int mi, int s, long ticks)
    {
        return new DateTime(y, mo, d, h, mi, s, DateTimeKind.Utc).AddTicks(ticks);
    }

    static P Scalars(string s, byte[] ansi, sbyte i8, byte u8, short i16, ushort u16, int i32, uint u32,
        long i64, ulong u64, float f32, double f64, int boolean, byte[] bin, string guid, ulong ptr,
        DateTime ft, int[] st, string sid, uint hex32, ulong hex64)
    {
        return new P().Str(s).Ansi(ansi).I8(i8).U8(u8).I16(i16).U16(u16).I32(i32).U32(u32).I64(i64).U64(u64)
            .F32(f32).F64(f64).I32(boolean).Bin(bin).Guid(guid).U64(ptr).FileTime(ft).SysTime(st).Sid(sid)
            .U32(hex32).U64(hex64);
    }

    // Every event to write, in event ID order, as (symbol, payload); a null payload
    // writes no data.
    static List<KeyValuePair<string, P>> Events()
    {
        List<KeyValuePair<string, P>> e = new List<KeyValuePair<string, P>>();
        Action<string, P> add = (symbol, p) => e.Add(new KeyValuePair<string, P>(symbol, p));
        DateTime stamp = Utc(2016, 7, 8, 18, 12, 51, 6816402);

        add("EVT_SCALARS", Scalars("hello", new byte[] { 0x63, 0x61, 0x66, 0xE9 }, -5, 200, -1234, 54321,
            -123456, 3000000000, -1234567890123, 12345678901234567890UL, 1.5f, 3.141592653589793, 1,
            Hex("DEADBEEF"), "01234567-89ab-cdef-0123-456789abcdef", 0x00007FF6A1B2C3D4UL,
            stamp, new int[] { 2019, 3, 5, 1, 12, 34, 56, 789 },
            "S-1-5-21-1004336348-1177238915-682003330-512", 0x1F, 0x80000000000000UL));
        add("EVT_SCALARS", Scalars("", new byte[0], sbyte.MinValue, 0, short.MinValue, 0, int.MinValue, 0,
            long.MinValue, 0, 0.0f, -0.0, 0, new byte[0], "00000000-0000-0000-0000-000000000000", 0,
            Utc(1601, 1, 1, 0, 0, 0, 0), new int[] { 1601, 1, 1, 1, 0, 0, 0, 0 }, "S-1-0-0", 0, 0));
        add("EVT_SCALARS", Scalars("x", new byte[] { 0x7E }, sbyte.MaxValue, byte.MaxValue, short.MaxValue,
            ushort.MaxValue, int.MaxValue, uint.MaxValue, long.MaxValue, ulong.MaxValue, float.MaxValue,
            double.MaxValue, 1, Hex("000102030405060708090A0B0C0D0E0F"), "ffffffff-ffff-ffff-ffff-ffffffffffff",
            ulong.MaxValue, Utc(9999, 12, 31, 23, 59, 59, 9999999), new int[] { 9999, 12, 5, 31, 23, 59, 59, 999 },
            "S-1-5-18", uint.MaxValue, ulong.MaxValue));

        foreach (string xml in new string[] {
            "<Root a=\"1\"><Child>text &amp; more</Child><Empty/></Root>",
            "plain text, not xml",
            "<Unclosed>" })
        {
            add("EVT_OUT_TYPES", new P().Str(xml).Ansi(Encoding.UTF8.GetBytes("\u00fcn\u00ef \U0001F600")).U8(0xAB)
                .U16(0xABCD).U32(0xDEADBEEF).U64(0x0123456789ABCDEFUL).U32(5).U32(0xC0000022)
                .I32(unchecked((int)0x80070005)).U32(1234).U16(0x5000).U32(0x0100007F)
                .Raw(Hex("00000000000000000000000000000001"))
                .Bin(Hex("02001F90C0A801010000000000000000"))
                .FileTime(stamp).FileTime(stamp));
        }

        string[][] texts = new string[][] {
            new string[] { "empty", "" },
            new string[] { "dash", "-" },
            new string[] { "spaces", "   " },
            new string[] { "trailing space", "value " },
            new string[] { "tab", "a\tb" },
            new string[] { "crlf", "line1\r\nline2" },
            new string[] { "lf", "line1\nline2" },
            new string[] { "cr", "line1\rline2" },
            new string[] { "c0 controls", "x\u0001\u0002\u0008\u000b\u000c\u000e\u001f\u007fy" },
            new string[] { "c1 controls", "x\u0080\u0085\u009fy" },
            new string[] { "markup", "<tag attr=\"v\">&amp; 'q'</tag>" },
            new string[] { "cdata end", "]]>" },
            new string[] { "lone high surrogate", "a\ud800b" },
            new string[] { "lone low surrogate", "a\udc00b" },
            new string[] { "reversed pair", "a\udc00\ud800b" },
            new string[] { "pair", "\ud83d\ude00 \u00fcn\u00efc\u00f6d\u00e9 \u65e5\u672c" },
            new string[] { "noncharacters", "x\ufffe\uffffy" },
            new string[] { "line separators", "x\u2028\u2029y" },
            new string[] { "bom", "\ufeffx" },
            new string[] { "format", "%1 %% %n %t" },
            new string[] { "backslash quote", "C:\\path\\\"quoted\"" },
        };
        foreach (string[] t in texts)
        {
            add("EVT_TEXT", new P().Str(t[0]).Str(t[1]));
        }

        foreach (string t in new string[] { "nul\u0000inside", "ends with nul\u0000", "\u0000", "", "plain" })
        {
            add("EVT_COUNTED", new P().Str("counted").U16((ushort)t.Length).Units(t));
        }

        add("EVT_ANSI", new P().Str("windows-1252 range").Ansi(Hex("808182838485868788898A8B8C8D8E8F909192939495969798999A9B9C9D9E9F")).Ansi(Hex("C3A9")));
        add("EVT_ANSI", new P().Str("high bytes").Ansi(Hex("A0E9FF")).Ansi(Hex("F09F9880")));
        add("EVT_ANSI", new P().Str("invalid utf-8").Ansi(Hex("41C328")).Ansi(Hex("41C328")));
        add("EVT_ANSI", new P().Str("truncated utf-8").Ansi(Hex("41E282")).Ansi(Hex("41E282")));
        add("EVT_ANSI", new P().Str("controls").Ansi(Hex("41011F42")).Ansi(Hex("41011F42")));

        add("EVT_EMPTY", null);

        long[] filetimes = new long[] {
            0, 1, 9999999, 10000000, 116444736000000000, 116444736000000001,
            stamp.ToFileTimeUtc(), Utc(2016, 7, 8, 18, 12, 51, 6816400).ToFileTimeUtc(), Utc(2016, 7, 8, 18, 12, 51, 6810000).ToFileTimeUtc(),
            Utc(2016, 7, 8, 18, 12, 51, 0).ToFileTimeUtc(), Utc(9999, 12, 31, 23, 59, 59, 9999999).ToFileTimeUtc(),
            Utc(9999, 12, 31, 23, 59, 59, 9999999).ToFileTimeUtc() + 1, 0x7FFFFFFFFFFFFFFF, -1 };
        foreach (long ft in filetimes)
        {
            add("EVT_FILETIME", new P().Str("0x" + ft.ToString("x16")).FileTime(ft));
        }

        int[][] systimes = new int[][] {
            new int[] { 2019, 3, 5, 1, 12, 34, 56, 789 },
            new int[] { 2019, 3, 0, 1, 12, 34, 56, 0 },
            new int[] { 2019, 3, 5, 1, 12, 34, 56, 999 },
            new int[] { 0, 0, 0, 0, 0, 0, 0, 0 },
            new int[] { 2019, 13, 0, 1, 0, 0, 0, 0 },
            new int[] { 2019, 2, 0, 30, 0, 0, 0, 0 },
            new int[] { 2019, 3, 0, 1, 24, 0, 0, 0 },
            new int[] { 2019, 3, 0, 1, 0, 0, 0, 1000 },
            new int[] { 30827, 12, 0, 31, 23, 59, 59, 999 },
        };
        foreach (int[] st in systimes)
        {
            add("EVT_SYSTIME", new P().Str(string.Join(",", Array.ConvertAll(st, x => x.ToString()))).SysTime(st));
        }

        Action<string, float, double> floats = (label, f, d) => add("EVT_FLOAT", new P().Str(label).F32(f).F64(d));
        floats("tenth", 0.1f, 0.1);
        floats("one", 1.0f, 1.0);
        floats("negative zero", -0.0f, -0.0);
        floats("nan", float.NaN, double.NaN);
        floats("infinity", float.PositiveInfinity, double.NegativeInfinity);
        floats("subnormal", 1e-45f, 5e-324);
        floats("large", 123456789f, 1e21);
        floats("small", 1e-7f, 1e-7);
        floats("max", float.MaxValue, double.MaxValue);
        floats("pi", 3.14159274f, 3.141592653589793);

        foreach (int b in new int[] { 0, 1, 2, -1, 0x10000 })
        {
            add("EVT_BOOL", new P().Str(b.ToString()).I32(b));
        }

        add("EVT_USER_DATA", new P().Str("v < w & x"));

        // Every input type as an array, with 3, 1 and 0 items; item(p, i) writes item i.
        Action<string, Action<P, int>> array = (symbol, item) =>
        {
            foreach (int n in new int[] { 3, 1, 0 })
            {
                P p = new P().U16((ushort)n);
                for (int i = 0; i < n; i++) item(p, i);
                add(symbol, p);
            }
        };
        int[][] systimes3 = new int[][] {
            new int[] { 2019, 3, 5, 1, 12, 34, 56, 789 },
            new int[] { 2020, 2, 6, 29, 0, 0, 0, 1 },
            new int[] { 1999, 12, 5, 31, 23, 59, 59, 999 } };
        array("EVT_A_UNICODESTRING", (p, i) => p.Str(new string[] { "a", "", "c,d" }[i]));
        array("EVT_A_ANSISTRING", (p, i) => p.Ansi(new string[] { "x", "", "y z" }[i]));
        array("EVT_A_INT8", (p, i) => p.I8(new sbyte[] { -1, 0, 1 }[i]));
        array("EVT_A_UINT8", (p, i) => p.U8(new byte[] { 0, 1, 255 }[i]));
        array("EVT_A_INT16", (p, i) => p.I16(new short[] { -2, 0, 2 }[i]));
        array("EVT_A_UINT16", (p, i) => p.U16(new ushort[] { 0, 1, 65535 }[i]));
        array("EVT_A_INT32", (p, i) => p.I32(new int[] { -3, 0, 3 }[i]));
        array("EVT_A_UINT32", (p, i) => p.U32(new uint[] { 0, 1, uint.MaxValue }[i]));
        array("EVT_A_INT64", (p, i) => p.I64(new long[] { -4, 0, 4 }[i]));
        array("EVT_A_UINT64", (p, i) => p.U64(new ulong[] { 0, 1, ulong.MaxValue }[i]));
        array("EVT_A_FLOAT", (p, i) => p.F32(new float[] { 0.5f, -1.25f, 1e10f }[i]));
        array("EVT_A_DOUBLE", (p, i) => p.F64(new double[] { 0.1, -2.5, 1e100 }[i]));
        array("EVT_A_BOOLEAN", (p, i) => p.I32(new int[] { 1, 0, 2 }[i]));
        array("EVT_A_BINARY", (p, i) => p.Raw(Hex(new string[] { "01020304", "A0B0C0D0", "FFFFFFFF" }[i])));
        array("EVT_A_GUID", (p, i) => p.Guid(new string[] { "01234567-89ab-cdef-0123-456789abcdef", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff" }[i]));
        array("EVT_A_POINTER", (p, i) => p.U64(new ulong[] { 0, 1, ulong.MaxValue }[i]));
        array("EVT_A_FILETIME", (p, i) => p.FileTime(new DateTime[] { stamp, Utc(1601, 1, 1, 0, 0, 0, 0), Utc(2020, 2, 29, 0, 0, 0, 1) }[i]));
        array("EVT_A_SYSTEMTIME", (p, i) => p.SysTime(systimes3[i]));
        array("EVT_A_SID", (p, i) => p.Sid(new string[] { "S-1-5-18", "S-1-5-19", "S-1-5-20" }[i]));
        array("EVT_A_HEXINT32", (p, i) => p.U32(new uint[] { 0, 0x1F, uint.MaxValue }[i]));
        array("EVT_A_HEXINT64", (p, i) => p.U64(new ulong[] { 0, 0x80000000000000UL, ulong.MaxValue }[i]));
        return e;
    }

    static EVENT_DESCRIPTOR Descriptor(Hashtable descriptors, string symbol)
    {
        ulong[] f = (ulong[])descriptors[symbol];
        if (f == null)
        {
            throw new ArgumentException(symbol + " is not an event of TypesTest.man");
        }
        EVENT_DESCRIPTOR d = new EVENT_DESCRIPTOR();
        d.Id = (ushort)f[0];
        d.Version = (byte)f[1];
        d.Channel = (byte)f[2];
        d.Level = (byte)f[3];
        d.Opcode = (byte)f[4];
        d.Task = (ushort)f[5];
        d.Keyword = f[6];
        return d;
    }

    // descriptors maps each event symbol to the 7 fields of its EVENT_DESCRIPTOR
    // (id, version, channel, level, opcode, task, keyword), from mc's header.
    public static int WriteAll(string providerGuid, Hashtable descriptors)
    {
        // Payloads write win:Pointer as 8 bytes; a 32-bit writer's events decode it as 4.
        if (IntPtr.Size != 8)
        {
            throw new InvalidOperationException("run from 64-bit PowerShell");
        }
        List<KeyValuePair<string, P>> events = Events();
        List<EVENT_DESCRIPTOR> ds = events.ConvertAll(ev => Descriptor(descriptors, ev.Key));
        Guid provider = new Guid(providerGuid);
        long handle;
        uint status = EventRegister(ref provider, IntPtr.Zero, IntPtr.Zero, out handle);
        if (status != 0)
        {
            throw new Win32Exception((int)status);
        }
        try
        {
            // The Event Log service enables the events shortly after the manifest is installed.
            // Until then EventWrite succeeds without logging anything.
            for (int i = 0; !ds.TrueForAll(d => EventEnabled(handle, ref d)); i++)
            {
                if (i == 120)
                {
                    throw new TimeoutException("the events were not enabled within 60 s");
                }
                Thread.Sleep(500);
            }
            for (int k = 0; k < events.Count; k++)
            {
                EVENT_DESCRIPTOR d = ds[k];
                P payload = events[k].Value;
                if (payload == null)
                {
                    status = EventWrite(handle, ref d, 0, null);
                }
                else
                {
                    byte[] bytes = payload.Bytes();
                    GCHandle pin = GCHandle.Alloc(bytes, GCHandleType.Pinned);
                    try
                    {
                        EVENT_DATA_DESCRIPTOR[] data = new EVENT_DATA_DESCRIPTOR[1];
                        data[0].Ptr = (ulong)pin.AddrOfPinnedObject().ToInt64();
                        data[0].Size = (uint)bytes.Length;
                        status = EventWrite(handle, ref d, 1, data);
                    }
                    finally
                    {
                        pin.Free();
                    }
                }
                if (status != 0)
                {
                    throw new Win32Exception((int)status, "EventWrite " + events[k].Key);
                }
            }
            return events.Count;
        }
        finally
        {
            EventUnregister(handle);
        }
    }
}
