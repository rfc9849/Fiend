# shellcheck shell=sh
case "$ARCH" in
    arm64) abi=arm64-v8a ;;
    x64) abi=x86_64 ;;
    *) abort "! Fiend root mode supports arm64 and x86_64 only" ;;
esac
[ -f "$MODPATH/bin/$abi/fiend-root" ] || abort "! The module has no helper for $abi"
mv "$MODPATH/bin/$abi/fiend-root" "$MODPATH/fiend-root"
rm -rf "$MODPATH/bin"
set_perm "$MODPATH/fiend-root" 0 0 0755
ui_print "- In the Fiend app, turn on Settings > Root mode and give Fiend root access"
