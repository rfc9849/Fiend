# ClientHello capture

A disguise is a ClientHello of a real app or device. To make one, capture it with [Wireshark](https://www.wireshark.org) and import it in the panel as a `.bin` file.

Turn off Fiend and other VPNs on the device first.

## Capture

1. Start a capture in Wireshark.
2. Open the app, so it connects to the site.
3. Stop the capture and enter this filter, with the site name of the app:

   ```
   tcp && tls.handshake.type == 1 && tls.handshake.extensions_server_name == "site.example"
   ```

4. Select a packet. In the packet details, open **Transport Layer Security** and select the **Record Layer** line.
5. Select **File > Export Packet Bytes** and save the file as `hello.bin`.

On Android, capture with [PCAPdroid](https://github.com/emanuele-f/PCAPdroid) in **PCAP file** mode, then open the file in Wireshark and continue from step 3.

## Import

In the panel, open **Disguises**, click **Add disguise**, select **Import a file** and choose `hello.bin`. Then select the disguise on the node page.

If the panel does not accept the file, make sure that you exported the **Record Layer** line. If it still fails, the app does not fit. Try a different app.
