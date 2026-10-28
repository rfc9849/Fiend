package dev.fiend.core.engine

import android.net.InetAddresses
import androidx.annotation.RequiresApi
import java.net.Inet6Address

@RequiresApi(29)
fun needsLocalNetworkAccess(host: String, bypassLan: Boolean): Boolean {
    if (bypassLan) return true
    val address = runCatching { InetAddresses.parseNumericAddress(host.removeSurrounding("[", "]")) }.getOrNull() ?: return false
    return address.isSiteLocalAddress || address.isLinkLocalAddress ||
        (address is Inet6Address && address.address[0].toInt() and 0xfe == 0xfc)
}
