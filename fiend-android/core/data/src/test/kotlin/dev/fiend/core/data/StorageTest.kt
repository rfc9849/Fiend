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
import dev.fiend.core.model.ThemeMode
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
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
            val original = repository.upsert(Profile("", "Local test", "127.0.0.1", 443, "42".repeat(32), template.id))
            val entity = database.profiles().get(original.id)!!
            assertFalse(entity.encryptedSecret.toString(Charsets.UTF_8).contains(original.secret))
            assertEquals(original.secret, withTimeout(5_000) { repository.profile(original.id).first { it != null } }!!.secret)
            val imported = repository.import(repository.shareLink(original)).getOrThrow()
            assertNotEquals(original.id, imported.id)
            assertEquals(original.secret, imported.secret)
            assertEquals(original.authority, imported.authority)
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
            repository.update { it.copy(themeMode = ThemeMode.Dark, dynamicColor = false, connectOnStart = true, ipv6 = false, splitTunnelApps = setOf("dev.fiend.test")) }
            val settings = withTimeout(5_000) { repository.settings.first { it.themeMode == ThemeMode.Dark } }
            assertFalse(settings.dynamicColor)
            assertTrue(settings.connectOnStart)
            assertFalse(settings.ipv6)
            assertEquals(setOf("dev.fiend.test"), settings.splitTunnelApps)
        } finally { scope.cancel() }
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
        val database = Room.databaseBuilder(context, ProfileDatabase::class.java, file.name).addMigrations(ProfileDatabase.MIGRATION_1_2).build()
        try {
            val restored = database.profiles().get("one")!!
            assertTrue(restored.encryptedSecret.contentEquals(encrypted))
            assertEquals("42".repeat(32), secretCipher.decrypt("one", restored.encryptedSecret))
            assertEquals(null, restored.templateJson)
        } finally { database.close(); context.deleteDatabase(file.name) }
    }
}
