// Isolated synthetic-window verification. Never enumerates mail or user data.
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Text;
using System.Windows.Automation;

class ReaderAccessibilityProbe {
    delegate bool Visitor(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(Visitor visitor, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window,StringBuilder name,int length);
    [MTAThread]
    static int Main(string[] args) {
        try {
            if (args.Length != 1) throw new Exception("Expected the fixture process id");
            uint wanted = uint.Parse(args[0]);
            IntPtr handle = IntPtr.Zero;
            EnumWindows(delegate(IntPtr window, IntPtr unused) {
                uint pid; GetWindowThreadProcessId(window, out pid);
                var name=new StringBuilder(128);GetClassName(window,name,name.Capacity);
                if (pid == wanted && name.ToString() == "Zed::Window") handle = window;
                return true;
            }, IntPtr.Zero);
            if (handle == IntPtr.Zero) throw new Exception("Fixture window missing");
            var root = AutomationElement.FromHandle(handle);
            var deadline = Stopwatch.StartNew();
            while (deadline.ElapsedMilliseconds < 15000) {
                var body = root.FindFirst(TreeScope.Descendants,
                    new PropertyCondition(AutomationElement.AutomationIdProperty, "mail-body"));
                object pattern;
                if (body != null && body.TryGetCurrentPattern(TextPattern.Pattern, out pattern)) {
                    string text = ((TextPattern)pattern).DocumentRange.GetText(-1);
                    if (text.Contains("Long header 中文") && text.Contains("Long tail 中文")) {
                        Console.WriteLine("UIA text pattern: full synthetic body available; characters=" + text.Length);
                        return 0;
                    }
                }
                Thread.Sleep(100);
            }
            throw new Exception("Document text pattern omitted the synthetic header or offscreen tail");
        } catch(Exception error) {
            Console.Error.WriteLine(error.Message); return 1;
        }
    }
}
