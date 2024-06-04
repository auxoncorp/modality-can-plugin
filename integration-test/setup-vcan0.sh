#!/bin/bash
# sudo apt-get install can-utils

if (( $EUID != 0 )); then
  echo "This script must be run as root"
  exit 1
fi

iface=vcan0
[ -n "$1" ] && iface=$1

# Load the 'vcan' kernel module
vcan_mod=$(lsmod | grep ^vcan)
if [ -z "${vcan_mod}" ]; then
    if ! modprobe vcan ; then
        echo "Unable to load the 'vcan' kernel module"
        exit 1
    fi
fi

ip link add type vcan
ip link set up "${iface}"

ip -details link show vcan0

exit 0
