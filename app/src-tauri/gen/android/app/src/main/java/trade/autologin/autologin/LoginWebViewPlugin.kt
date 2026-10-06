package trade.autologin.autologin

import android.annotation.SuppressLint
import android.app.Activity
import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.KeyCharacterMap
import android.view.View
import android.view.ViewGroup
import android.webkit.CookieManager
import android.webkit.RenderProcessGoneDetail
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebStorage
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.FrameLayout
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayInputStream
import java.io.File
import java.io.FileOutputStream
import java.util.regex.Pattern

@InvokeArg
class OpenArgs {
  var visible: Boolean = false
  var intercept: String? = null
}

@InvokeArg
class UrlArgs {
  lateinit var url: String
}

@InvokeArg
class ScriptArgs {
  lateinit var script: String
}

@InvokeArg
class TextArgs {
  lateinit var text: String
}

@InvokeArg
class PathArgs {
  lateinit var path: String
}

/**
 * Drives one broker login page in a plain Android WebView, for the Rust
 * step engine. The WebView has no Tauri bridge, so broker pages can't reach
 * the app. Only one login page exists at a time; cookies and storage are
 * wiped when it opens and closes so accounts never share a session.
 */
@TauriPlugin
class LoginWebViewPlugin(private val activity: Activity) : Plugin(activity) {
  private var container: FrameLayout? = null
  private var webView: WebView? = null
  @Volatile private var intercept: Pattern? = null
  @Volatile private var captured: String? = null
  private var pendingLoad: Invoke? = null
  private var loadStarted = false
  private val pendingEvals = mutableSetOf<Invoke>()

  /**
   * Runs [block] on the UI thread. Any exception rejects the call instead of
   * crashing the app (uncaught exceptions on the UI thread are fatal).
   */
  private fun onUi(invoke: Invoke, block: () -> Unit) {
    activity.runOnUiThread {
      try {
        block()
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.javaClass.simpleName)
      }
    }
  }

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.parseArgs(OpenArgs::class.java)
    val pattern = try {
      args.intercept?.let { Pattern.compile(it) }
    } catch (e: Exception) {
      return invoke.reject("callback pattern isn't valid here: ${e.message}")
    }
    onUi(invoke) {
      teardown()
      clearBrowsingData()
      intercept = pattern
      captured = null
      val view = createWebView()
      val frame = FrameLayout(activity)
      frame.addView(view, matchParent())
      frame.addView(hideButton(), FrameLayout.LayoutParams(
        ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT,
        android.view.Gravity.TOP or android.view.Gravity.END,
      ))
      val root = activity.findViewById<ViewGroup>(android.R.id.content)
      // Hidden pages sit behind the app's own UI: still laid out and
      // rendered (so visibility checks work), just not seen.
      if (args.visible) root.addView(frame, matchParent()) else root.addView(frame, 0, matchParent())
      container = frame
      webView = view
      invoke.resolve()
    }
  }

  @Command
  fun load(invoke: Invoke) {
    val args = invoke.parseArgs(UrlArgs::class.java)
    onUi(invoke) {
      val view = webView ?: return@onUi invoke.reject(NOT_OPEN)
      pendingLoad?.resolve()
      pendingLoad = invoke
      // Only a page that starts after this call can finish it, so a late
      // onPageFinished from the previous document doesn't end it early.
      loadStarted = false
      view.loadUrl(args.url)
    }
  }

  // Runs a script inside the broker login page. Only the Rust step engine
  // calls this (selector lookup, fill, manifest `eval` steps); no capability
  // grants these commands to the app's own UI.
  @Command
  fun eval(invoke: Invoke) {
    val args = invoke.parseArgs(ScriptArgs::class.java)
    onUi(invoke) {
      val view = webView ?: return@onUi invoke.reject(NOT_OPEN)
      // The callback receives the result JSON-encoded ("null" if none). It
      // never fires if the page dies, so teardown rejects what's left.
      pendingEvals.add(invoke)
      view.evaluateJavascript(args.script) { json ->
        if (pendingEvals.remove(invoke)) invoke.resolve(JSObject().put("value", json))
      }
    }
  }

  @Command
  fun url(invoke: Invoke) {
    onUi(invoke) {
      val view = webView ?: return@onUi invoke.reject(NOT_OPEN)
      invoke.resolve(JSObject().put("value", view.url ?: ""))
    }
  }

  @Command
  fun captured(invoke: Invoke) {
    invoke.resolve(JSObject().put("value", captured))
  }

  /** Real key events into the focused element, for inputs that ignore `fill`. */
  @Command
  fun typeText(invoke: Invoke) {
    val args = invoke.parseArgs(TextArgs::class.java)
    onUi(invoke) {
      val view = webView ?: return@onUi invoke.reject(NOT_OPEN)
      view.requestFocus()
      val events = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(args.text.toCharArray())
        ?: return@onUi invoke.reject("these characters can't be typed as key presses")
      events.forEach { view.dispatchKeyEvent(it) }
      invoke.resolve()
    }
  }

  @Command
  fun screenshot(invoke: Invoke) {
    val args = invoke.parseArgs(PathArgs::class.java)
    onUi(invoke) {
      val view = webView ?: return@onUi invoke.reject(NOT_OPEN)
      if (view.width == 0 || view.height == 0) return@onUi invoke.reject("login page has no size yet")
      val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
      try {
        view.draw(Canvas(bitmap))
        FileOutputStream(File(args.path)).use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
      } finally {
        bitmap.recycle()
      }
      invoke.resolve()
    }
  }

  @Command
  fun close(invoke: Invoke) {
    onUi(invoke) {
      teardown()
      clearBrowsingData()
      invoke.resolve()
    }
  }

  @SuppressLint("SetJavaScriptEnabled")
  private fun createWebView(): WebView {
    val view = WebView(activity)
    view.settings.apply {
      javaScriptEnabled = true
      domStorageEnabled = true
      allowFileAccess = false
      allowContentAccess = false
      setSupportMultipleWindows(false)
      // Look like mobile Chrome: some login pages refuse embedded WebViews.
      userAgentString = userAgentString.replace("; wv)", ")").replace(Regex(" Version/[\\d.]+"), "")
    }
    view.isFocusableInTouchMode = true
    view.webViewClient = LoginClient()
    return view
  }

  private fun hideButton(): Button = Button(activity).apply {
    text = "Hide"
    setOnClickListener {
      val frame = container ?: return@setOnClickListener
      val root = activity.findViewById<ViewGroup>(android.R.id.content)
      root.removeView(frame)
      root.addView(frame, 0, matchParent())
      visibility = View.GONE
    }
  }

  private inner class LoginClient : WebViewClient() {
    override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
      request.isForMainFrame && capture(request.url.toString())

    // Backstop for redirects that never reach shouldOverrideUrlLoading.
    override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? {
      if (!request.isForMainFrame || !capture(request.url.toString())) return null
      return WebResourceResponse("text/plain", "utf-8", 204, "No Content", emptyMap(), ByteArrayInputStream(ByteArray(0)))
    }

    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
      loadStarted = true
    }

    override fun onPageFinished(view: WebView, url: String) {
      if (loadStarted) finishLoad(null)
    }

    // A killed renderer would otherwise take the whole app down with it.
    override fun onRenderProcessGone(view: WebView, detail: RenderProcessGoneDetail): Boolean {
      if (view === webView) teardown()
      return true
    }

    override fun onReceivedError(view: WebView, request: WebResourceRequest, error: WebResourceError) {
      if (request.isForMainFrame && captured == null) finishLoad("page failed to load: ${error.description}")
    }
  }

  /** Records the callback URL and blocks it: the broker's code is single-use. */
  private fun capture(url: String): Boolean {
    val pattern = intercept ?: return false
    if (!pattern.matcher(url).find()) return false
    captured = url
    activity.runOnUiThread { finishLoad(null) }
    return true
  }

  private fun finishLoad(error: String?) {
    val invoke = pendingLoad ?: return
    pendingLoad = null
    if (error == null) invoke.resolve() else invoke.reject(error)
  }

  private fun teardown() {
    pendingLoad?.reject("login page closed")
    pendingLoad = null
    pendingEvals.forEach { it.reject("login page closed") }
    pendingEvals.clear()
    container?.let { (it.parent as? ViewGroup)?.removeView(it) }
    webView?.apply {
      stopLoading()
      clearCache(true)
      destroy()
    }
    container = null
    webView = null
    intercept = null
  }

  private fun clearBrowsingData() {
    CookieManager.getInstance().removeAllCookies(null)
    CookieManager.getInstance().flush()
    WebStorage.getInstance().deleteAllData()
  }

  private fun matchParent() =
    FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)

  private companion object {
    const val NOT_OPEN = "login page is not open"
  }
}
