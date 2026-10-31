package dev.fiend.core.engine

import dev.fiend.core.engine.bindings.ProfileConfig
import dev.fiend.core.engine.bindings.FiendException
import dev.fiend.core.engine.bindings.bundledTemplates
import dev.fiend.core.engine.bindings.loadTemplate
import dev.fiend.core.engine.bindings.makeShareLink
import dev.fiend.core.engine.bindings.parseShareLink
import dev.fiend.core.engine.bindings.templateDomain
import dev.fiend.core.engine.bindings.validateProfile
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class BindingsTest {
    @Test
    fun shareLinkRoundTripUsesRust() {
        val template = bundledTemplates().single()
        val profile = ProfileConfig("Local test", listOf("127.0.0.1:443", "192.0.2.10:8443"), "42".repeat(32), template.templateJson)
        validateProfile(profile)
        val link = makeShareLink(profile)
        val restored = parseShareLink(link)
        assertTrue(link.startsWith("fiend://"))
        assertTrue(link.length < 1200)
        assertEquals(profile.name, restored.name)
        assertEquals(profile.servers, restored.servers)
        assertEquals(profile.secret, restored.secret)
        assertEquals(link, makeShareLink(restored))
    }

    @Test
    fun templateFilesLoadThroughRust() {
        val template = bundledTemplates().single()
        assertEquals("www.wikipedia.org", template.domain)
        val loaded = loadTemplate(template.templateJson.toByteArray())
        assertEquals(template.domain, loaded.domain)
        assertEquals(template.domain, templateDomain(loaded.templateJson))
        validateProfile(ProfileConfig("Local test", listOf("127.0.0.1:443"), "42".repeat(32), loaded.templateJson))
        assertThrows(FiendException::class.java) { loadTemplate("not a template".toByteArray()) }
    }
}
