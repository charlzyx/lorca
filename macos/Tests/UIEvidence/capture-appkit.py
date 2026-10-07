#!/usr/bin/env python3
"""Capture unchanged production AppKit controllers in a synthetic, network-free host."""
from pathlib import Path
import subprocess, tempfile
ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / '.github/evidence/issue-76'
with tempfile.TemporaryDirectory(prefix='lorca-attention-capture-') as scratch:
    scratch = Path(scratch)
    controls = (ROOT/'macos/Sources/Lorca/Design/Controls.swift').read_text()
    # Use the production label/stack helpers; the remaining Controls helpers are unused.
    (scratch/'Build.swift').write_text(controls[:controls.index('    static func imageButton(')]+'}\n')
    binary = scratch/'AttentionCapture'
    subprocess.run(['swiftc','-swift-version','5','-parse-as-library','-framework','AppKit',
        str(scratch/'Build.swift'),str(ROOT/'macos/Sources/Lorca/Sheets/SheetViewController.swift'),
        str(ROOT/'macos/Sources/Lorca/Attention/AttentionViewController.swift'),
        str(ROOT/'macos/Tests/UIEvidence/AttentionCapture.swift'),'-o',str(binary)],check=True)
    subprocess.run([str(binary),str(OUT/'fixture.json'),str(OUT)],check=True)
