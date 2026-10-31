package dev.fiend.core.data

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import androidx.room.Room
import com.google.crypto.tink.Aead
import com.google.crypto.tink.KeysetHandle
import com.google.crypto.tink.aead.AeadConfig2026
import com.google.crypto.tink.aead.PredefinedAeadParameters
import dev.fiend.core.data.storage.PreferencesStore
import dev.fiend.core.data.storage.ProfileDatabase
import dev.fiend.core.data.storage.SecretCipher
import dev.fiend.core.engine.bindings.bundledTemplates
import dev.fiend.core.model.Profile
import dev.fiend.core.model.RuleCounts
import dev.fiend.core.model.RuleFileSpec
import dev.fiend.core.model.RuleKind
import dev.fiend.core.model.RuleSubscription
import dev.fiend.core.model.ThemeMode
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import com.sun.net.httpserver.HttpServer
import java.io.File
import java.net.InetSocketAddress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class StorageTest {
    private fun cipher(): SecretCipher {
        val configuration = AeadConfig2026.get()
        val handle = KeysetHandle.generateNew(PredefinedAeadParameters.AES256_GCM, configuration)
        val aead = handle.getPrimitive(configuration, Aead::class.java)
        return SecretCipher { aead }
    }

    @Test
    fun tinkRoundTripIsRandomAndBoundToTheProfileId() {
        val cipher = cipher()
        val secret = "42".repeat(32)
        val encrypted = cipher.encrypt("one", secret)
        assertEquals(secret, cipher.decrypt("one", encrypted))
        assertFalse(encrypted.toString(Charsets.UTF_8).contains(secret))
        assertFalse(encrypted.contentEquals(cipher.encrypt("one", secret)))
        assertTrue(runCatching { cipher.decrypt("two", encrypted) }.isFailure)
        encrypted[encrypted.lastIndex] = (encrypted.last().toInt() xor 1).toByte()
        assertTrue(runCatching { cipher.decrypt("one", encrypted) }.isFailure)
    }

    @Test
    fun roomStoresEncryptedBytesAndRepositoryImportsRealLinks() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val database = Room.inMemoryDatabaseBuilder(context, ProfileDatabase::class.java).build()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val store = PreferencesStore(context, scope)
            val repository = RoomProfileRepository(database, cipher(), store, RealLogRepository(), scope)
            repository.awaitReady()
            val template = bundledTemplates().single()
            val original = repository.upsert(Profile("", "Local test", "127.0.0.1", 443, "42".repeat(32), template.id, moreServers = listOf("192.0.2.10:8443", "[2001:db8::1]:443")))
            val entity = database.profiles().get(original.id)!!
            assertFalse(entity.encryptedSecret.toString(Charsets.UTF_8).contains(original.secret))
            assertEquals(original.secret, withTimeout(5_000) { repository.profile(original.id).first { it != null } }!!.secret)
            val imported = repository.import(repository.shareLink(original)).getOrThrow()
            assertNotEquals(original.id, imported.id)
            assertEquals(original.secret, imported.secret)
            assertEquals(original.authority, imported.authority)
            assertEquals(original.servers, imported.servers)
            assertEquals(original.servers, withTimeout(5_000) { repository.profile(imported.id).first { it != null } }!!.servers)
            repository.setActive(imported.id)
            assertEquals(imported.id, repository.activeProfileId.value)
            repository.delete(imported.id)
            assertEquals(original.id, repository.activeProfileId.value)
            assertTrue(repository.import("fiend://invalid").isFailure)
        } finally { scope.cancel(); database.close() }
    }

    @Test
    fun settingsRoundTripKeepsSavedValues() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val store = PreferencesStore(context, scope)
            val repository = RealSettingsRepository(store, RealLogRepository(), scope)
            repository.awaitReady()
            repository.update { it.copy(themeMode = ThemeMode.Dark, dynamicColor = false, connectOnStart = true, ipv6 = false, splitTunnelApps = setOf("dev.fiend.test"), directRules = "domain:ru\nip:1.2.3.0/24", blockRules = "domain:ads.example", proxyRules = "domain:blocked.ru", dpiRules = "domain:youtube.com", subscriptions = listOf(RuleSubscription("one", "https://example.com/list.srs", RuleKind.Proxy, updatedAtMs = 5, entries = 7)), directResolver = "1.1.1.1", mtu = 9999) }
            val settings = withTimeout(5_000) { repository.settings.first { it.themeMode == ThemeMode.Dark } }
            assertEquals("domain:ru\nip:1.2.3.0/24", settings.directRules)
            assertEquals("domain:ads.example", settings.blockRules)
            assertEquals("domain:blocked.ru", settings.proxyRules)
            assertEquals("domain:youtube.com", settings.dpiRules)
            assertEquals(listOf(RuleSubscription("one", "https://example.com/list.srs", RuleKind.Proxy, updatedAtMs = 5, entries = 7)), settings.subscriptions)
            assertEquals("1.1.1.1", settings.directResolver)
            assertEquals(9000, settings.mtu)
            assertFalse(settings.dynamicColor)
            assertTrue(settings.connectOnStart)
            assertFalse(settings.ipv6)
            assertEquals(setOf("dev.fiend.test"), settings.splitTunnelApps)
        } finally { scope.cancel() }
    }

    @Test
    fun subscriptionsRejectBadLinksAndHaveNoFilesBeforeTheFirstDownload() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        try {
            val settings = RealSettingsRepository(PreferencesStore(context, scope), RealLogRepository(), scope)
            settings.awaitReady()
            val repository = RealSubscriptionRepository(context, settings, RealRulesRepository(context))
            assertTrue(repository.add("ftp://example.com/list.srs", RuleKind.Direct).isFailure)
            assertTrue(repository.add("", RuleKind.Direct).isFailure)
            assertTrue(settings.settings.value.subscriptions.isEmpty())
            assertTrue(repository.ruleFiles().isEmpty())
        } finally { scope.cancel() }
    }

    @Test
    fun subscriptionsDownloadValidateAndReplaceAFile() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/list.txt") { exchange ->
            val body = "domain:example.ru\n".toByteArray()
            exchange.sendResponseHeaders(200, body.size.toLong())
            exchange.responseBody.use { it.write(body) }
        }
        server.start()
        try {
            val settings = RealSettingsRepository(PreferencesStore(context, scope), RealLogRepository(), scope)
            settings.awaitReady()
            val repository = RealSubscriptionRepository(context, settings, RealRulesRepository(context))
            val subscription = repository.add("http://127.0.0.1:${server.address.port}/list.txt", RuleKind.Direct).getOrThrow()
            assertNull(subscription.error)
            assertEquals(1, subscription.entries)
            val file = File(context.filesDir, "rules/${subscription.id}.bin")
            assertTrue(file.isFile)
            assertEquals("domain:example.ru\n", file.readText())
            assertEquals(listOf(RuleFileSpec(file.absolutePath, RuleKind.Direct)), repository.ruleFiles())
            repository.remove(subscription.id)
            assertFalse(file.exists())
            assertTrue(settings.settings.value.subscriptions.isEmpty())
        } finally { server.stop(0); scope.cancel() }
    }

    @Test
    fun rulesAreValidatedByRustAndImportIsLimited() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val repository = RealRulesRepository(context)
        val counts = repository.validate("domain:a.ru\nfull:b.ru\nkeyword:x\nip:10.0.0.0/8", true).getOrThrow()
        assertEquals(RuleCounts(1, 1, 1, 1), counts)
        assertEquals(4, counts.total)
        assertTrue(repository.validate("ip:10.0.0.0/8", false).isFailure)
        assertTrue(repository.validate("bogus:x", true).isFailure)
        assertTrue(repository.validateResolver("").isSuccess)
        assertTrue(repository.validateResolver("9.9.9.9:53").isSuccess)
        assertTrue(repository.validateResolver("dns.example").isFailure)
        val small = java.io.File(context.cacheDir, "small.txt").apply { writeText("domain:example.ru\n") }
        assertEquals("domain:example.ru\n", repository.read(android.net.Uri.fromFile(small).toString()).getOrThrow())
        val large = java.io.File(context.cacheDir, "large.txt").apply { writeBytes(ByteArray(RulesRepository.MAX_IMPORT_BYTES + 1) { 'a'.code.toByte() }) }
        assertTrue(repository.read(android.net.Uri.fromFile(large).toString()).isFailure)
        val binary = java.io.File(context.cacheDir, "binary.txt").apply { writeBytes(byteArrayOf(0xff.toByte(), 0xfe.toByte(), 0x00)) }
        assertTrue(repository.read(android.net.Uri.fromFile(binary).toString()).isFailure)
    }

    @Test
    fun migrationFromVersionOneKeepsEncryptedRows() = runBlocking {
        val context = RuntimeEnvironment.getApplication() as Context
        val file = context.getDatabasePath("migration-test.db")
        file.parentFile!!.mkdirs()
        val secretCipher = cipher()
        val encrypted = secretCipher.encrypt("one", "42".repeat(32))
        SQLiteDatabase.openOrCreateDatabase(file, null).use { database ->
            database.execSQL("CREATE TABLE profiles (id TEXT NOT NULL PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL, encrypted_secret BLOB NOT NULL, template_id TEXT NOT NULL, country_code TEXT, latency_ms INTEGER, last_connected_at INTEGER, handshake_timeout_ms INTEGER NOT NULL, max_connections INTEGER NOT NULL, udp_idle_ms INTEGER NOT NULL, udp_keepalive_ms INTEGER NOT NULL, max_udp_flows INTEGER NOT NULL)")
            database.execSQL("INSERT INTO profiles VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 10000, 1024, 60000, 20000, 64)", arrayOf("one", "Local test", "127.0.0.1", 443, encrypted, "captured-wikipedia-v1"))
            database.version = 1
        }
        val database = Room.databaseBuilder(context, ProfileDatabase::class.java, file.name).addMigrations(ProfileDatabase.MIGRATION_1_2, ProfileDatabase.MIGRATION_2_3).build()
        try {
            val restored = database.profiles().get("one")!!
            assertTrue(restored.encryptedSecret.contentEquals(encrypted))
            assertEquals("42".repeat(32), secretCipher.decrypt("one", restored.encryptedSecret))
            assertEquals(null, restored.templateJson)
            assertEquals("", restored.moreServers)
        } finally { database.close(); context.deleteDatabase(file.name) }
    }
}
