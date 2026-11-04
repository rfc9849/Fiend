#!/system/bin/sh
MODDIR=${0%/*}
[ -x "$MODDIR/fiend-root" ] && "$MODDIR/fiend-root" cleanup
