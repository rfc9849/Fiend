package dev.fiend.core.data.storage

import com.google.crypto.tink.Aead
import com.google.crypto.tink.integration.android.AndroidKeystore

internal class SecretCipher(private val factory: () -> Aead) {
    private val aead by lazy(factory)

    @Synchronized
    fun encrypt(id: String, secret: String): ByteArray {
        val bytes = secret.toByteArray(Charsets.UTF_8)
        return try { aead.encrypt(bytes, associatedData(id)) } finally { bytes.fill(0) }
    }

    @Synchronized
    fun decrypt(id: String, ciphertext: ByteArray): String {
        val bytes = aead.decrypt(ciphertext, associatedData(id))
        return try { bytes.toString(Charsets.UTF_8) } finally { bytes.fill(0) }
    }

    private fun associatedData(id: String) = "fiend/profile/$id/secret/v1".toByteArray(Charsets.UTF_8)

    companion object {
        fun androidKeystore() = SecretCipher {
            val alias = "fiend_profile_master_v1"
            if (!AndroidKeystore.hasKey(alias)) AndroidKeystore.generateNewAes256GcmKey(alias)
            AndroidKeystore.getAead(alias)
        }
    }
}
