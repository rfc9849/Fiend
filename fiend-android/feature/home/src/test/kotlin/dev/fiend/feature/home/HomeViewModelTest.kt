package dev.fiend.feature.home

import app.cash.turbine.test
import dev.fiend.core.data.ProfileRepository
import dev.fiend.core.data.TunnelController
import dev.fiend.core.model.ConnectStep
import dev.fiend.core.model.FingerprintTemplate
import dev.fiend.core.model.Profile
import dev.fiend.core.model.TunnelState
import dev.fiend.core.model.TunnelStats
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class HomeViewModelTest {
    @Before fun prepare() { Dispatchers.setMain(StandardTestDispatcher()) }
    @After fun finish() { Dispatchers.resetMain() }

    @Test
    fun tunnelErrorsAndStatsReachTheScreen() = runTest {
        val controller = TestTunnel()
        val model = HomeViewModel(TestProfiles(), controller)
        model.uiState.test {
            var initial = awaitItem()
            if (initial.activeProfile == null) initial = awaitItem()
            assertEquals("one", initial.activeProfile?.id)
            controller.state.value = TunnelState.Connecting("one", ConnectStep.Handshake)
            assertTrue(awaitItem().tunnel is TunnelState.Connecting)
            controller.state.value = TunnelState.Connected("one", 123)
            assertEquals(123L, (awaitItem().tunnel as TunnelState.Connected).sinceEpochMs)
            controller.stats.value = TunnelStats(receivedBytes = 4096, activeFlows = 2)
            assertEquals(4096L, awaitItem().stats.receivedBytes)
            controller.state.value = TunnelState.Failed("one", "VPN permission was not granted")
            assertEquals("VPN permission was not granted", (awaitItem().tunnel as TunnelState.Failed).reason)
            model.toggle()
            assertEquals("one", controller.requested)
        }
    }

    private class TestTunnel : TunnelController {
        override val state = MutableStateFlow<TunnelState>(TunnelState.Disconnected)
        override val stats = MutableStateFlow(TunnelStats.Empty)
        override val permissionRequired = MutableStateFlow(false)
        var requested: String? = null
        override fun connect(profileId: String) { requested = profileId }
        override fun disconnect() { state.value = TunnelState.Disconnected }
        override fun permissionResult(granted: Boolean) { }
    }

    private class TestProfiles : ProfileRepository {
        override val profiles = MutableStateFlow(listOf(Profile("one", "Test", "127.0.0.1", 443, "42".repeat(32), "test")))
        override val activeProfileId = MutableStateFlow<String?>("one")
        override val templates = listOf(FingerprintTemplate("test", "Test", "TLS"))
        override fun profile(id: String) = profiles.map { list -> list.firstOrNull { it.id == id } }
        override suspend fun setActive(id: String) { activeProfileId.value = id }
        override suspend fun upsert(profile: Profile) = profile
        override suspend fun delete(id: String) { }
        override suspend fun duplicate(id: String): Profile? = null
        override suspend fun import(link: String): Result<Profile> = Result.failure(IllegalArgumentException())
        override suspend fun measureLatency(id: String) = 0
        override fun shareLink(profile: Profile) = ""
    }
}
