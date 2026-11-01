package dev.fiend.core.data.storage

import androidx.room.AutoMigration
import androidx.room.ColumnInfo
import androidx.room.Dao
import androidx.room.Database
import androidx.room.DeleteColumn
import androidx.room.Entity
import androidx.room.PrimaryKey
import androidx.room.Query
import androidx.room.RoomDatabase
import androidx.room.Upsert
import androidx.room.migration.AutoMigrationSpec
import kotlinx.coroutines.flow.Flow

@Entity(tableName = "profiles")
internal data class ProfileEntity(
    @PrimaryKey val id: String,
    val name: String,
    val host: String,
    val port: Int,
    @ColumnInfo(name = "encrypted_secret", typeAffinity = ColumnInfo.BLOB) val encryptedSecret: ByteArray,
    @ColumnInfo(name = "template_id") val templateId: String,
    @ColumnInfo(name = "template_json") val templateJson: String?,
    @ColumnInfo(name = "country_code") val countryCode: String?,
    @ColumnInfo(name = "latency_ms") val latencyMs: Int?,
    @ColumnInfo(name = "last_connected_at") val lastConnectedAt: Long?,
    @ColumnInfo(name = "handshake_timeout_ms") val handshakeTimeoutMs: Int,
    @ColumnInfo(name = "max_connections") val maxConnections: Int,
    @ColumnInfo(name = "udp_idle_ms") val udpIdleMs: Int,
    @ColumnInfo(name = "max_udp_flows") val maxUdpFlows: Int,
    @ColumnInfo(name = "more_servers", defaultValue = "") val moreServers: String = "",
)

@Dao
internal interface ProfileDao {
    @Query("SELECT * FROM profiles ORDER BY name COLLATE NOCASE, id") fun observe(): Flow<List<ProfileEntity>>
    @Query("SELECT * FROM profiles ORDER BY name COLLATE NOCASE, id") suspend fun all(): List<ProfileEntity>
    @Query("SELECT * FROM profiles WHERE id = :id") suspend fun get(id: String): ProfileEntity?
    @Upsert suspend fun upsert(profile: ProfileEntity)
    @Query("DELETE FROM profiles WHERE id = :id") suspend fun delete(id: String)
    @Query("UPDATE profiles SET latency_ms = :latency WHERE id = :id") suspend fun latency(id: String, latency: Int)
    @Query("UPDATE profiles SET last_connected_at = :time WHERE id = :id") suspend fun connected(id: String, time: Long)
}

@Database(
    entities = [ProfileEntity::class],
    version = 4,
    exportSchema = true,
    autoMigrations = [AutoMigration(from = 3, to = 4, spec = ProfileDatabase.DropUdpKeepalive::class)],
)
internal abstract class ProfileDatabase : RoomDatabase() {
    abstract fun profiles(): ProfileDao

    @DeleteColumn(tableName = "profiles", columnName = "udp_keepalive_ms")
    class DropUdpKeepalive : AutoMigrationSpec
}
