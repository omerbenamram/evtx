// Writes the events of TypesTest.man through EventWrite, with every payload
// built byte by byte (see generate-types-sample.ps1).
//
// Compiled through Add-Type in Windows PowerShell 5.1, so it sticks to C# 5.
using System;
using System.Collections;
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

    [DllImport("advapi32.dll")]
    static extern bool EventProviderEnabled(long regHandle, byte level, ulong keyword);

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
        public P SysTime(int year, int month, int dow, int day, int hour, int minute, int second, int ms)
        {
            foreach (int v in new int[] { year, month, dow, day, hour, minute, second, ms })
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
            .F32(f32).F64(f64).I32(boolean).Bin(bin).Guid(guid).U64(ptr).FileTime(ft)
            .SysTime(st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]).Sid(sid).U32(hex32).U64(hex64);
    }

    static P Arrays(string[] strings, sbyte[] i8, byte[] u8, short[] i16, ushort[] u16,
        int[] i32, uint[] u32, long[] i64, ulong[] u64, float[] f32, double[] f64, int[] bools,
        string[] guids, ulong[] ptrs, DateTime[] fts, int[][] sts, string[] sids, uint[] hex32, ulong[] hex64)
    {
        P p = new P().U16((ushort)strings.Length);
        foreach (string v in strings) p.Str(v);
        foreach (sbyte v in i8) p.I8(v);
        foreach (byte v in u8) p.U8(v);
        foreach (short v in i16) p.I16(v);
        foreach (ushort v in u16) p.U16(v);
        foreach (int v in i32) p.I32(v);
        foreach (uint v in u32) p.U32(v);
        foreach (long v in i64) p.I64(v);
        foreach (ulong v in u64) p.U64(v);
        foreach (float v in f32) p.F32(v);
        foreach (double v in f64) p.F64(v);
        foreach (int v in bools) p.I32(v);
        foreach (string v in guids) p.Guid(v);
        foreach (ulong v in ptrs) p.U64(v);
        foreach (DateTime v in fts) p.FileTime(v);
        foreach (int[] st in sts) p.SysTime(st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]);
        foreach (string v in sids) p.Sid(v);
        foreach (uint v in hex32) p.U32(v);
        foreach (ulong v in hex64) p.U64(v);
        return p;
    }

    // Every event to write, as (symbol, payload); a null payload writes no data.
    static ArrayList Events()
    {
        ArrayList e = new ArrayList();
        Action<string, P> add = (symbol, p) => e.Add(new object[] { symbol, p });

        add("EVT_SCALARS", Scalars("hello", new byte[] { 0x63, 0x61, 0x66, 0xE9 }, -5, 200, -1234, 54321,
            -123456, 3000000000, -1234567890123, 12345678901234567890UL, 1.5f, 3.141592653589793, 1,
            Hex("DEADBEEF"), "01234567-89ab-cdef-0123-456789abcdef", 0x00007FF6A1B2C3D4UL,
            Utc(2016, 7, 8, 18, 12, 51, 6816402), new int[] { 2019, 3, 5, 1, 12, 34, 56, 789 },
            "S-1-5-21-1004336348-1177238915-682003330-512", 0x1F, 0x80000000000000UL));
        add("EVT_SCALARS", Scalars("", new byte[0], sbyte.MinValue, 0, short.MinValue, 0, int.MinValue, 0,
            long.MinValue, 0, 0.0f, -0.0, 0, new byte[0], "00000000-0000-0000-0000-000000000000", 0,
            Utc(1601, 1, 1, 0, 0, 0, 0), new int[] { 1601, 1, 1, 1, 0, 0, 0, 0 }, "S-1-0-0", 0, 0));
        add("EVT_SCALARS", Scalars("x", new byte[] { 0x7E }, sbyte.MaxValue, byte.MaxValue, short.MaxValue,
            ushort.MaxValue, int.MaxValue, uint.MaxValue, long.MaxValue, ulong.MaxValue, float.MaxValue,
            double.MaxValue, 1, Hex("000102030405060708090A0B0C0D0E0F"), "ffffffff-ffff-ffff-ffff-ffffffffffff",
            ulong.MaxValue, Utc(9999, 12, 31, 23, 59, 59, 9999999), new int[] { 9999, 12, 5, 31, 23, 59, 59, 999 },
            "S-1-5-18", uint.MaxValue, ulong.MaxValue));

        int[] st1 = new int[] { 2019, 3, 5, 1, 12, 34, 56, 789 };
        int[] st2 = new int[] { 2020, 2, 6, 29, 0, 0, 0, 1 };
        int[] st3 = new int[] { 1999, 12, 5, 31, 23, 59, 59, 999 };
        add("EVT_ARRAYS", Arrays(new string[] { "a", "", "c,d" },
            new sbyte[] { -1, 0, 1 }, new byte[] { 0, 1, 255 }, new short[] { -2, 0, 2 }, new ushort[] { 0, 1, 65535 },
            new int[] { -3, 0, 3 }, new uint[] { 0, 1, uint.MaxValue }, new long[] { -4, 0, 4 },
            new ulong[] { 0, 1, ulong.MaxValue }, new float[] { 0.5f, -1.25f, 1e10f }, new double[] { 0.1, -2.5, 1e100 },
            new int[] { 1, 0, 1 },
            new string[] { "01234567-89ab-cdef-0123-456789abcdef", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff" },
            new ulong[] { 0, 1, ulong.MaxValue },
            new DateTime[] { Utc(2016, 7, 8, 18, 12, 51, 6816402), Utc(1601, 1, 1, 0, 0, 0, 0), Utc(2020, 2, 29, 0, 0, 0, 1) },
            new int[][] { st1, st2, st3 }, new string[] { "S-1-5-18", "S-1-5-19", "S-1-5-20" },
            new uint[] { 0, 0x1F, uint.MaxValue }, new ulong[] { 0, 0x80000000000000UL, ulong.MaxValue }));
        add("EVT_ARRAYS", Arrays(new string[] { "only" }, new sbyte[] { 7 }, new byte[] { 7 },
            new short[] { 7 }, new ushort[] { 7 }, new int[] { 7 }, new uint[] { 7 }, new long[] { 7 }, new ulong[] { 7 },
            new float[] { 7f }, new double[] { 7 }, new int[] { 0 }, new string[] { "01234567-89ab-cdef-0123-456789abcdef" },
            new ulong[] { 7 }, new DateTime[] { Utc(2016, 7, 8, 18, 12, 51, 6816402) }, new int[][] { st1 },
            new string[] { "S-1-5-18" }, new uint[] { 7 }, new ulong[] { 7 }));
        add("EVT_ARRAYS", Arrays(new string[0], new sbyte[0], new byte[0], new short[0], new ushort[0],
            new int[0], new uint[0], new long[0], new ulong[0], new float[0], new double[0], new int[0], new string[0],
            new ulong[0], new DateTime[0], new int[0][], new string[0], new uint[0], new ulong[0]));

        // The same arrays one type per event, with 3, 1 and 0 items.
        object[][] arrays = new object[][] {
            new object[] { "EVT_A_UNICODESTRING", (Action<P, int>)((p, i) => p.Str(new string[] { "a", "", "c,d" }[i])) },
            new object[] { "EVT_A_INT8", (Action<P, int>)((p, i) => p.I8(new sbyte[] { -1, 0, 1 }[i])) },
            new object[] { "EVT_A_UINT8", (Action<P, int>)((p, i) => p.U8(new byte[] { 0, 1, 255 }[i])) },
            new object[] { "EVT_A_INT16", (Action<P, int>)((p, i) => p.I16(new short[] { -2, 0, 2 }[i])) },
            new object[] { "EVT_A_UINT16", (Action<P, int>)((p, i) => p.U16(new ushort[] { 0, 1, 65535 }[i])) },
            new object[] { "EVT_A_INT32", (Action<P, int>)((p, i) => p.I32(new int[] { -3, 0, 3 }[i])) },
            new object[] { "EVT_A_UINT32", (Action<P, int>)((p, i) => p.U32(new uint[] { 0, 1, uint.MaxValue }[i])) },
            new object[] { "EVT_A_INT64", (Action<P, int>)((p, i) => p.I64(new long[] { -4, 0, 4 }[i])) },
            new object[] { "EVT_A_UINT64", (Action<P, int>)((p, i) => p.U64(new ulong[] { 0, 1, ulong.MaxValue }[i])) },
            new object[] { "EVT_A_FLOAT", (Action<P, int>)((p, i) => p.F32(new float[] { 0.5f, -1.25f, 1e10f }[i])) },
            new object[] { "EVT_A_DOUBLE", (Action<P, int>)((p, i) => p.F64(new double[] { 0.1, -2.5, 1e100 }[i])) },
            new object[] { "EVT_A_BOOLEAN", (Action<P, int>)((p, i) => p.I32(new int[] { 1, 0, 2 }[i])) },
            new object[] { "EVT_A_GUID", (Action<P, int>)((p, i) => p.Guid(new string[] { "01234567-89ab-cdef-0123-456789abcdef", "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff" }[i])) },
            new object[] { "EVT_A_POINTER", (Action<P, int>)((p, i) => p.U64(new ulong[] { 0, 1, ulong.MaxValue }[i])) },
            new object[] { "EVT_A_FILETIME", (Action<P, int>)((p, i) => p.FileTime(new DateTime[] { Utc(2016, 7, 8, 18, 12, 51, 6816402), Utc(1601, 1, 1, 0, 0, 0, 0), Utc(2020, 2, 29, 0, 0, 0, 1) }[i])) },
            new object[] { "EVT_A_SYSTEMTIME", (Action<P, int>)((p, i) => { int[] st = new int[][] { st1, st2, st3 }[i]; p.SysTime(st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]); }) },
            new object[] { "EVT_A_SID", (Action<P, int>)((p, i) => p.Sid(new string[] { "S-1-5-18", "S-1-5-19", "S-1-5-20" }[i])) },
            new object[] { "EVT_A_HEXINT32", (Action<P, int>)((p, i) => p.U32(new uint[] { 0, 0x1F, uint.MaxValue }[i])) },
            new object[] { "EVT_A_HEXINT64", (Action<P, int>)((p, i) => p.U64(new ulong[] { 0, 0x80000000000000UL, ulong.MaxValue }[i])) },
        };
        foreach (object[] array in arrays)
        {
            foreach (int n in new int[] { 3, 1, 0 })
            {
                P p = new P().U16((ushort)n);
                for (int i = 0; i < n; i++) ((Action<P, int>)array[1])(p, i);
                add((string)array[0], p);
            }
        }

        foreach (string[] ansis in new string[][] { new string[] { "x", "", "y z" }, new string[] { "one" }, new string[0] })
        {
            P p = new P().U16((ushort)ansis.Length);
            foreach (string v in ansis) p.Ansi(v);
            add("EVT_ANSI_ARRAY", p);
        }

        add("EVT_USER_DATA", new P().Str("v < w & x"));

        add("EVT_BINARY_ARRAY", new P().U16(2).Raw(Hex("01020304")).Raw(Hex("A0B0C0D0")));
        add("EVT_BINARY_ARRAY", new P().U16(0));

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
                .FileTime(Utc(2016, 7, 8, 18, 12, 51, 6816402)).FileTime(Utc(2016, 7, 8, 18, 12, 51, 6816402)));
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
            0, 1, 9999999, 10000000, 116444736000000000, 116444736000000001, 131125111716816402,
            Utc(2016, 7, 8, 18, 12, 51, 6816400).ToFileTimeUtc(), Utc(2016, 7, 8, 18, 12, 51, 6810000).ToFileTimeUtc(),
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
            add("EVT_SYSTIME", new P().Str(string.Join(",", Array.ConvertAll(st, x => x.ToString())))
                .SysTime(st[0], st[1], st[2], st[3], st[4], st[5], st[6], st[7]));
        }

        object[][] floats = new object[][] {
            new object[] { "tenth", 0.1f, 0.1 },
            new object[] { "one", 1.0f, 1.0 },
            new object[] { "negative zero", -0.0f, -0.0 },
            new object[] { "nan", float.NaN, double.NaN },
            new object[] { "infinity", float.PositiveInfinity, double.NegativeInfinity },
            new object[] { "subnormal", 1e-45f, 5e-324 },
            new object[] { "large", 123456789f, 1e21 },
            new object[] { "small", 1e-7f, 1e-7 },
            new object[] { "max", float.MaxValue, double.MaxValue },
            new object[] { "pi", 3.14159274f, 3.141592653589793 },
        };
        foreach (object[] f in floats)
        {
            add("EVT_FLOAT", new P().Str((string)f[0]).F32((float)f[1]).F64((double)f[2]));
        }

        foreach (int b in new int[] { 0, 1, 2, -1, 0x10000 })
        {
            add("EVT_BOOL", new P().Str(b.ToString()).I32(b));
        }
        return e;
    }

    // descriptors maps each event symbol to the 7 fields of its EVENT_DESCRIPTOR
    // (id, version, channel, level, opcode, task, keyword), from mc's header.
    public static int WriteAll(string providerGuid, Hashtable descriptors)
    {
        Guid provider = new Guid(providerGuid);
        long handle;
        uint status = EventRegister(ref provider, IntPtr.Zero, IntPtr.Zero, out handle);
        if (status != 0)
        {
            throw new System.ComponentModel.Win32Exception((int)status);
        }
        try
        {
            // The Event Log service enables the provider shortly after the manifest is installed.
            for (int i = 0; i < 120 && !EventProviderEnabled(handle, 4, 0); i++)
            {
                Thread.Sleep(500);
            }
            int written = 0;
            foreach (object[] ev in Events())
            {
                string symbol = (string)ev[0];
                ulong[] f = (ulong[])descriptors[symbol];
                EVENT_DESCRIPTOR d = new EVENT_DESCRIPTOR();
                d.Id = (ushort)f[0];
                d.Version = (byte)f[1];
                d.Channel = (byte)f[2];
                d.Level = (byte)f[3];
                d.Opcode = (byte)f[4];
                d.Task = (ushort)f[5];
                d.Keyword = f[6];
                P payload = (P)ev[1];
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
                    throw new System.ComponentModel.Win32Exception((int)status, "EventWrite " + symbol);
                }
                written++;
            }
            return written;
        }
        finally
        {
            EventUnregister(handle);
        }
    }
}
