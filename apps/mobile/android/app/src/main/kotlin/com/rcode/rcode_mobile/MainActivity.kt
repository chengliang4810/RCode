package com.rcode.rcode_mobile

import android.content.ClipboardManager
import android.content.Context
import android.provider.OpenableColumns
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import java.io.ByteArrayOutputStream

class MainActivity : FlutterActivity() {
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "com.rcode.mobile/image_clipboard")
            .setMethodCallHandler { call, result ->
                if (call.method != "readImage") {
                    result.notImplemented()
                    return@setMethodCallHandler
                }
                // Clipboard access occurs only in response to the Dart Paste image action.
                val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                val clip = try {
                    clipboard.primaryClip
                } catch (_: Exception) {
                    result.error("clipboard_unavailable", "Unable to read clipboard image", null)
                    return@setMethodCallHandler
                }
                val imageDescription = clip?.description?.filterMimeTypes("image/*")?.isNotEmpty() == true
                val uri = (0 until (clip?.itemCount ?: 0))
                    .mapNotNull { clip?.getItemAt(it)?.uri }
                    .firstOrNull { uri ->
                        uri.scheme in listOf("content", "file", "android.resource") &&
                            (imageDescription || runCatching {
                                contentResolver.getType(uri)?.startsWith("image/") == true
                            }.getOrDefault(false))
                    }
                if (uri == null) {
                    result.success(null)
                    return@setMethodCallHandler
                }
                Thread {
                    try {
                        val maxBytes = 24 * 1024 * 1024
                        val bytes = contentResolver.openInputStream(uri)?.use { stream ->
                            val output = ByteArrayOutputStream()
                            val buffer = ByteArray(8192)
                            while (true) {
                                val count = stream.read(buffer)
                                if (count < 0) break
                                if (count > maxBytes - output.size()) {
                                    throw ImageTooLarge()
                                }
                                output.write(buffer, 0, count)
                            }
                            output.toByteArray()
                        } ?: throw IllegalStateException("No image data")
                        var name = "clipboard.png"
                        runCatching {
                            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                                ?.use { cursor ->
                                    if (cursor.moveToFirst()) {
                                        val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                                        if (column >= 0) {
                                            name = cursor.getString(column)?.take(255) ?: name
                                        }
                                    }
                                }
                        }
                        runOnUiThread { result.success(mapOf("bytes" to bytes, "name" to name)) }
                    } catch (_: ImageTooLarge) {
                        runOnUiThread { result.error("image_too_large", "Image exceeds 24 MiB", null) }
                    } catch (_: Exception) {
                        runOnUiThread { result.error("clipboard_unavailable", "Unable to read clipboard image", null) }
                    }
                }.start()
            }
    }

    private class ImageTooLarge : Exception()
}
