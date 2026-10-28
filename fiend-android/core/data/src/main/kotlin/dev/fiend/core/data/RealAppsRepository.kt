package dev.fiend.core.data

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.core.graphics.drawable.toBitmap
import dagger.hilt.android.qualifiers.ApplicationContext
import dev.fiend.core.model.InstalledApp
import java.io.ByteArrayOutputStream
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withContext

@Singleton
class RealAppsRepository @Inject constructor(@ApplicationContext private val context: Context) : AppsRepository {
    override val apps = callbackFlow {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) { trySend(Unit) }
        }
        val filter = IntentFilter().apply { addAction(Intent.ACTION_PACKAGE_ADDED); addAction(Intent.ACTION_PACKAGE_REMOVED); addAction(Intent.ACTION_PACKAGE_REPLACED); addDataScheme("package") }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        trySend(Unit)
        awaitClose { context.unregisterReceiver(receiver) }
    }.conflate().map {
        withContext(Dispatchers.IO) {
            val manager = context.packageManager
            val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
            val resolved = if (Build.VERSION.SDK_INT >= 33) manager.queryIntentActivities(launcher, PackageManager.ResolveInfoFlags.of(0))
                else {
                    @Suppress("DEPRECATION")
                    manager.queryIntentActivities(launcher, 0)
                }
            resolved.distinctBy { it.activityInfo.packageName }.map { item ->
                val info = item.activityInfo.applicationInfo
                val icon = runCatching {
                    ByteArrayOutputStream().use { bytes -> info.loadIcon(manager).toBitmap(96, 96).compress(Bitmap.CompressFormat.PNG, 100, bytes); bytes.toByteArray() }
                }.getOrNull()
                InstalledApp(info.packageName, item.loadLabel(manager).toString(), info.flags and ApplicationInfo.FLAG_SYSTEM != 0, icon)
            }.sortedBy { it.label.lowercase() }
        }
    }
}
