#!/bin/bash

set -e

if [[ $EUID -ne 0 ]]; then
    echo "This script must be run as root to load modules"
    exit 1
fi

pci_modules=$(lspci -k 2>/dev/null | grep "Kernel modules:" | awk -F': ' '{print $2}' | tr ', ' '\n\n' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | sort -u)

if [[ -z "$pci_modules" ]]; then
    echo "No kernel modules found from lspci"
    exit 0
fi

echo "Found PCI kernel modules:"
echo "$pci_modules" | sed 's/^/  /'
echo

# Get list of currently loaded modules
echo "Checking loaded modules..."
loaded_modules=$(lsmod | awk 'NR>1 {print $1}')

# Load modules that aren't already loaded
modules_loaded=0
modules_skipped=0

for module in $pci_modules; do
    # Skip empty lines
    [[ -z "$module" ]] && continue

    # Check if module is already loaded
    if echo "$loaded_modules" | grep -qw "$module"; then
        echo "  [SKIP] $module (already loaded)"
        modules_skipped=$((modules_skipped+1))
    else
        echo "  [LOAD] $module"
        if modprobe -b "$module" 2>/dev/null; then
            modules_loaded=$((modules_loaded+1))
        else
            echo "    Warning: Failed to load $module"
        fi
    fi
done

echo
echo "Summary: $modules_loaded module(s) loaded, $modules_skipped module(s) already loaded"
