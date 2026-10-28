package dev.fiend.feature.profiles.list

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import android.content.pm.PackageManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.compose.CameraXViewfinder
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.SurfaceRequest
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import dev.fiend.core.designsystem.icon.FiendIcons
import dev.fiend.feature.profiles.R
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.suspendCancellableCoroutine

@Composable
internal fun QrScanner(onResult: (String) -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var permission by remember { mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) }
    val askPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { permission = it }
    DisposableEffect(lifecycle) {
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) permission = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED }
        lifecycle.lifecycle.addObserver(observer)
        onDispose { lifecycle.lifecycle.removeObserver(observer) }
    }
    var surface by remember { mutableStateOf<SurfaceRequest?>(null) }
    var cameraError by remember { mutableStateOf(false) }
    val result by rememberUpdatedState(onResult)
    val provider by produceState<ProcessCameraProvider?>(null, permission) {
        if (permission) {
            value = runCatching {
                val future = ProcessCameraProvider.getInstance(context)
                suspendCancellableCoroutine { continuation ->
                    future.addListener({ if (continuation.isActive) continuation.resumeWith(runCatching { future.get() }) }, ContextCompat.getMainExecutor(context))
                }
            }.getOrElse { cameraError = true; null }
        }
    }
    DisposableEffect(provider, permission, lifecycle) {
        val camera = provider
        val executor = Executors.newSingleThreadExecutor()
        val preview = Preview.Builder().build()
        val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
        val found = AtomicBoolean(false)
        val active = AtomicBoolean(true)
        val reader = MultiFormatReader().apply { setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE), DecodeHintType.TRY_HARDER to true)) }
        if (camera != null && permission) {
            preview.setSurfaceProvider { surface = it }
            analysis.setAnalyzer(executor) { image ->
                try {
                    if (!found.get()) {
                        val plane = image.planes[0]
                        val buffer = plane.buffer.duplicate()
                        val base = buffer.position()
                        val data = ByteArray(image.width * image.height)
                        for (y in 0 until image.height) for (x in 0 until image.width) data[y * image.width + x] = buffer.get(base + y * plane.rowStride + x * plane.pixelStride)
                        val source = PlanarYUVLuminanceSource(data, image.width, image.height, 0, 0, image.width, image.height, false)
                        val decoded = reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
                        if (found.compareAndSet(false, true)) ContextCompat.getMainExecutor(context).execute { if (active.get()) result(decoded) }
                    }
                } catch (_: Exception) { }
                finally { reader.reset(); image.close() }
            }
            runCatching { camera.bindToLifecycle(lifecycle, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis) }.onFailure { cameraError = true }
        }
        onDispose { active.set(false); analysis.clearAnalyzer(); camera?.unbind(preview, analysis); executor.shutdown(); surface?.willNotProvideSurface() }
    }
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Scaffold(topBar = {
            TopAppBar(title = { Text(stringResource(R.string.profiles_add_scan)) }, navigationIcon = { IconButton(onClick = onDismiss) { Icon(FiendIcons.ArrowBack, contentDescription = stringResource(R.string.scan_close)) } })
        }) { padding ->
            Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                if (!permission || cameraError) {
                    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center, modifier = Modifier.padding(24.dp)) {
                        Text(stringResource(if (cameraError) R.string.scan_camera_error else R.string.scan_permission))
                        Spacer(Modifier.height(16.dp))
                        if (!permission) {
                            Button(onClick = { askPermission.launch(Manifest.permission.CAMERA) }) { Text(stringResource(R.string.scan_allow)) }
                            Button(onClick = { context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }) { Text(stringResource(R.string.scan_settings)) }
                        }
                    }
                } else surface?.let { CameraXViewfinder(surfaceRequest = it, modifier = Modifier.fillMaxSize()) }
            }
        }
    }
}
