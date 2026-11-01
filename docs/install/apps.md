# Apps

Files are in the [latest release](https://github.com/rfc9849/fiend/releases/latest). Replace `0.1.0` below with its version.

## Android

Install `fiend-0.1.0-android.apk`. Android 8.0 or later.

## macOS

macOS 13 or later. Open `fiend-0.1.0-macos-universal.dmg` and drag Fiend to Applications.

Fiend is not notarized by Apple, so macOS blocks the first launch. On macOS 15 and later, open **System Settings**, then **Privacy & Security**, and click **Open Anyway** next to the Fiend message. On macOS 13 and 14, Control-click Fiend in Applications and choose **Open**.

Then click **Install service** in the app.

## Windows

Extract `fiend-0.1.0-windows-amd64.zip` (or `-arm64.zip`), run `Fiend.exe` and click **Install service**. The build is unsigned, so SmartScreen may ask: **More info**, then **Run anyway**.

## Linux

Needs a working Vulkan driver.

```sh
sudo apt install ./fiend_0.1.0-1_amd64.deb
# or
sudo dnf install ./fiend-0.1.0-1.x86_64.rpm
```

Log out and back in once after the install.
