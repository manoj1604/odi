#!/bin/bash
# Firecracker timings, on Linux with /dev/kvm (here: the
# Lima VM, nested virtualization on an Apple M4). Makes ~/fc with Firecracker,
# a guest kernel and root filesystems from Firecracker's CI, then:
#
#   cd ~/fc && python3 coldboot.py   # launch to the guest's first program
#   cd ~/fc && python3 snap.py       # snapshot a running guest, restore it
#   cd ~/fc && python3 app.py        # Express in a microVM: cold boot and
#                                    # restore, to the first HTTP answer
#
# app.py wants Node 22 at ~/node and an Express app at EXPRESS (a folder
# with server.js, listening on $PORT, and its node_modules).
set -eu
EXPRESS=${EXPRESS:?the Express app folder}
HERE=$(cd "$(dirname "$0")" && pwd); mkdir -p ~/fc && cd ~/fc
TAG=$(curl -fsSLI -o /dev/null -w "%{url_effective}" https://github.com/firecracker-microvm/firecracker/releases/latest | sed "s#.*/##")
curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/$TAG/firecracker-$TAG-aarch64.tgz" | tar xz
cp "release-$TAG-aarch64/firecracker-$TAG-aarch64" firecracker
CI=http://spec.ccfc.min.s3.amazonaws.com/firecracker-ci/v1.15/aarch64
curl -fsSL -o vmlinux $CI/vmlinux-6.1.155
curl -fsSL -o ubuntu.squashfs $CI/ubuntu-24.04.squashfs
sudo apt-get install -y squashfs-tools >/dev/null
sudo rm -rf root && sudo unsquashfs -q -d root ubuntu.squashfs
printf '#!/bin/sh\necho ODI_BOOTED\nwhile true; do echo tick; sleep 0.02; done\n' | sudo tee root/sbin/odi-boot >/dev/null
sudo chmod +x root/sbin/odi-boot
truncate -s 1G rootfs.ext4 && sudo mkfs.ext4 -q -d root -F rootfs.ext4
cp rootfs.ext4 rootfs-node.ext4 && truncate -s 1536M rootfs-node.ext4 && resize2fs rootfs-node.ext4 >/dev/null
sudo mkdir -p /mnt/fcroot && sudo mount -o loop rootfs-node.ext4 /mnt/fcroot
sudo cp ~/node/bin/node /mnt/fcroot/usr/local/bin/node && sudo cp -R "$EXPRESS" /mnt/fcroot/app
sudo tee /mnt/fcroot/sbin/odi-boot >/dev/null <<'INIT'
#!/bin/sh
mount -t proc proc /proc 2>/dev/null; mount -t tmpfs tmpfs /tmp 2>/dev/null
ip link set lo up; ip addr add 172.16.0.2/24 dev eth0; ip link set eth0 up
echo ODI_BOOTED
cd /app && PORT=8080 NODE_ENV=production exec /usr/local/bin/node server.js
INIT
sudo chmod +x /mnt/fcroot/sbin/odi-boot && sudo umount /mnt/fcroot && sudo rm -rf root ubuntu.squashfs
sudo ip link del tap0 2>/dev/null || true
sudo ip tuntap add dev tap0 mode tap user "$(whoami)" && sudo ip addr add 172.16.0.1/24 dev tap0 && sudo ip link set tap0 up
cp "$HERE"/*.py "$HERE"/*.json ~/fc/
