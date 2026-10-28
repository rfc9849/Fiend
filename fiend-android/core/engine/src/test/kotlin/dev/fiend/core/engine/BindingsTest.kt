package dev.fiend.core.engine

import dev.fiend.core.engine.bindings.ProfileConfig
import dev.fiend.core.engine.bindings.bundledTemplates
import dev.fiend.core.engine.bindings.makeShareLink
import dev.fiend.core.engine.bindings.parseShareLink
import dev.fiend.core.engine.bindings.validateProfile
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class BindingsTest {
    @Test
    fun shareLinkRoundTripUsesRust() {
        val template = bundledTemplates().single()
        val profile = ProfileConfig("Local test", "127.0.0.1:443", "42".repeat(32), template.templateJson)
        validateProfile(profile)
        val link = makeShareLink(profile)
        val restored = parseShareLink(link)
        assertTrue(link.startsWith("fiend://v1/"))
        assertEquals(profile.name, restored.name)
        assertEquals(profile.server, restored.server)
        assertEquals(profile.secret, restored.secret)
        assertEquals(link, makeShareLink(restored))
    }
}
