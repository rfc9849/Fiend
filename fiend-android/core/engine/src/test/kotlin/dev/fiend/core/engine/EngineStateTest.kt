package dev.fiend.core.engine

import app.cash.turbine.test
import dev.fiend.core.engine.bindings.TunnelState as NativeState
import dev.fiend.core.model.TunnelState
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EngineStateTest {
    @Test
    fun nativeStatesMapToUiAndOldSessionsCannotReconnect() = runTest {
        val engine = EngineState()
        engine.state.test {
            assertEquals(TunnelState.Disconnected, awaitItem())
            val old = engine.begin("one")
            assertTrue(awaitItem() is TunnelState.Connecting)
            engine.update(old, NativeState.Connected)
            assertEquals("one", (awaitItem() as TunnelState.Connected).profileId)
            val current = engine.begin("two")
            assertEquals("two", (awaitItem() as TunnelState.Connecting).profileId)
            engine.update(old, NativeState.Failed("Old failure"))
            expectNoEvents()
            engine.update(current, NativeState.Failed("Socket protection failed"))
            assertEquals("Socket protection failed", (awaitItem() as TunnelState.Failed).reason)
            engine.disconnected()
            assertEquals(TunnelState.Disconnected, awaitItem())
            engine.update(current, NativeState.Connected)
            expectNoEvents()
        }
    }
}
