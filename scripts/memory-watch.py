#!/usr/bin/env python3
"""Bound a diagnostic child process; never targets an existing user application."""
import json, os, subprocess, sys, time
from pathlib import Path
root=Path(__file__).resolve().parent.parent
logpath=root/'validation/memory-offscreen.log'
logpath.parent.mkdir(parents=True, exist_ok=True)
with logpath.open('w') as log:
    child=subprocess.Popen(sys.argv[1:],stdout=log,stderr=subprocess.STDOUT,cwd=root)
    start=time.monotonic(); peak=0; sampled=False; stopped=None; samples=[]
    try:
        while child.poll() is None:
            rss=subprocess.run(['ps','-o','rss=','-p',str(child.pid)],capture_output=True,text=True).stdout.strip()
            if rss:
                rss=int(rss); peak=max(peak,rss); samples.append([round(time.monotonic()-start,2),rss])
                if rss>160*1024 and not sampled:
                    subprocess.Popen(['sample',str(child.pid),'1','10','-file',str(root/'validation/private/memory-stack.txt')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    sampled=True
                if rss>512*1024:
                    stopped='512 MiB memory ceiling'; child.kill(); break
            if time.monotonic()-start>int(os.environ.get("MEMORY_TEST_DEADLINE", "40")):
                stopped='test deadline'; child.kill(); break
            time.sleep(.05)
    finally:
        if child.poll() is None: child.kill()
        code=child.wait()
    report={'exit_code':code,'peak_rss_mib':round(peak/1024,2),'stopped':stopped,'elapsed_seconds':round(time.monotonic()-start,2),'samples':samples}
    (root/'validation/memory-offscreen.json').write_text(json.dumps(report,indent=2))
    print({k:v for k,v in report.items() if k!='samples'})
    sys.exit(1 if stopped else code)
