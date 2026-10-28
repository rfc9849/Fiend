package dev.fiend.core.data

import dev.fiend.core.engine.needsLocalNetworkAccess
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class NetworkPermissionTest {
    @Test
    fun localServersAndLanBypassNeedLocalNetworkAccess() {
        for (host in listOf("10.0.2.2", "192.168.1.1", "172.16.0.1", "169.254.1.1", "[fd42::1]", "fe80::1")) assertTrue(needsLocalNetworkAccess(host, false))
        assertTrue(needsLocalNetworkAccess("1.1.1.1", true))
        assertFalse(needsLocalNetworkAccess("1.1.1.1", false))
        assertFalse(needsLocalNetworkAccess("2606:4700:4700::1111", false))
        assertFalse(needsLocalNetworkAccess("example.com", false))
    }
}
