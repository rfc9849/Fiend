package dev.fiend.feature.profiles.detail

import com.google.zxing.BinaryBitmap
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class QrCodeTest {
    @Test
    fun realQrDecodesToTheOriginalLink() {
        val link = "fiend://v1/test-payload"
        val matrix = qrMatrix(link)!!
        val scale = 8
        val width = matrix.width * scale
        val pixels = IntArray(width * width) { index -> if (matrix[(index % width) / scale, (index / width) / scale]) 0xff000000.toInt() else 0xffffffff.toInt() }
        val bitmap = BinaryBitmap(HybridBinarizer(RGBLuminanceSource(width, width, pixels)))
        assertEquals(link, QRCodeReader().decode(bitmap).text)
    }

    @Test
    fun oversizedPayloadHasAClearNoQrState() {
        assertNull(qrMatrix("x".repeat(10_000)))
    }
}
