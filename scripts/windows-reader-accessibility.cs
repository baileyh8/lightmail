// Isolated synthetic-window verification. Never enumerates mail or user data.
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
using System.Text;
using System.Windows.Automation;
using System.Windows.Automation.Text;

class ReaderAccessibilityProbe {
    delegate bool Visitor(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(Visitor visitor, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr window,StringBuilder name,int length);
    [MTAThread]
    static int Main(string[] args) {
        try {
            if (args.Length < 1 || args.Length > 2) throw new Exception("Expected the fixture process id and optional probe mode");
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
            if (args.Length == 2 && args[1] == "--link-dialog") {
                var address = root.FindFirst(TreeScope.Descendants,
                    new PropertyCondition(AutomationElement.AutomationIdProperty, "reader-link-address"));
                object value;
                if (address == null || !address.TryGetCurrentPattern(ValuePattern.Pattern, out value))
                    throw new Exception("Confirmation URL did not expose a value pattern");
                var pattern = (ValuePattern)value;
                if (!pattern.Current.IsReadOnly)
                    throw new Exception("Confirmation URL did not expose its read-only state");
                if (pattern.Current.Value != "https://example.com/synthetic-link?t=a%2Bb%3D&next=%2Fdocs#section")
                    throw new Exception("Confirmation URL changed signed URL bytes; fixture value=" + pattern.Current.Value);
                Console.WriteLine("UIA link confirmation: complete original URL and read-only field verified");
                return 0;
            }
            var deadline = Stopwatch.StartNew();
            while (deadline.ElapsedMilliseconds < 15000) {
                var body = root.FindFirst(TreeScope.Descendants,
                    new PropertyCondition(AutomationElement.AutomationIdProperty, "mail-body"));
                object pattern;
                if (body != null && body.TryGetCurrentPattern(TextPattern.Pattern, out pattern)) {
                    string text = ((TextPattern)pattern).DocumentRange.GetText(-1);
                    if (text.Contains("Long header 中文") && text.Contains("Long tail 中文")) {
                        var provider=(TextPattern)pattern;
                        var selected=provider.GetSelection();
                        if(selected.Length!=1 || !selected[0].GetText(-1).Contains("Long header 中文") || !selected[0].GetText(-1).Contains("Long tail 中文")) throw new Exception("UIA did not expose the app's cross-region selection");
                        var tail=provider.DocumentRange.Clone();
                        tail.MoveEndpointByRange(TextPatternRangeEndpoint.Start,tail,TextPatternRangeEndpoint.End);
                        tail.MoveEndpointByUnit(TextPatternRangeEndpoint.Start,TextUnit.Character,-("Long tail 中文\n".Length));
                        tail.Select();
                        var selectionDeadline=Stopwatch.StartNew();bool selectedTail=false;
                        while(selectionDeadline.ElapsedMilliseconds<5000) {
                            selected=provider.GetSelection();
                            if(selected.Length==1 && selected[0].GetText(-1).Trim()=="Long tail 中文") {selectedTail=true;break;}
                            Thread.Sleep(100);
                        }
                        if(!selectedTail) throw new Exception("UIA selection action did not update the app");
                        Console.WriteLine("UIA text pattern: full body, exposed selection and selection action verified; characters=" + text.Length);
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
