#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Reject a loader and Scout rootfs built with different kernel releases."""

import argparse
from pathlib import Path
import re
import subprocess

import pefile


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output_dir", type=Path)
output_dir = parser.parse_args().output_dir

# The loader's kernel keeps running after the switch into Scout's rootfs.
with pefile.PE(str(output_dir / "loader.efi"), fast_load=True) as loader:
    kernel_release = ""
    for section in loader.sections:
        if section.Name.rstrip(b"\0") == b".uname":
            size = min(section.Misc_VirtualSize, section.SizeOfRawData)
            kernel_release = section.get_data(length=size).rstrip(b"\0").decode("ascii")
            break

if not kernel_release:
    raise SystemExit("ERROR: Loader UKI is missing its kernel release (.uname)")

listing = subprocess.run(
    ["unsquashfs", "-l", "-no-progress", str(output_dir / "scout.squashfs"), "usr/lib/modules"],
    check=True,
    stdout=subprocess.PIPE,
    text=True,
)
module_releases = set(re.findall(r"^squashfs-root/usr/lib/modules/([^/\n]+)$", listing.stdout, re.MULTILINE))
if module_releases != {kernel_release}:
    raise SystemExit(
        f"ERROR: Loader kernel {kernel_release} does not match Scout modules {sorted(module_releases)}; "
        "rebuild both images together"
    )

print(f"Scout and loader kernel releases match: {kernel_release}")
