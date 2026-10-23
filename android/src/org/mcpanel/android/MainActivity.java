package org.mcpanel.android;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.net.Uri;
import android.os.*;
import android.webkit.*;
import android.widget.*;
import java.io.*;
import java.net.*;

/** Local panel UI. Untrusted pages never receive app privileges or a JS bridge. */
public final class MainActivity extends Activity {
    private WebView web;
    private TextView message;
    private Button start;
    private Button log;
    private String origin;
    private ValueCallback<Uri[]> upload;
    private String downloadUrl;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable poll = new Runnable() {
        @Override public void run() {
            if (HostService.url != null && !HostService.url.equals(origin)) {
                origin = HostService.url;
                web.setVisibility(android.view.View.VISIBLE);
                message.setVisibility(android.view.View.GONE);
                web.loadUrl(origin);
            } else if (HostService.url == null) {
                web.setVisibility(android.view.View.GONE);
                message.setText(HostService.status);
                message.setVisibility(android.view.View.VISIBLE);
            }
            start.setVisibility(HostService.active ? android.view.View.GONE : android.view.View.VISIBLE);
            log.setVisibility(HostService.url == null ? android.view.View.VISIBLE : android.view.View.GONE);
            handler.postDelayed(this, 500);
        }
    };

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setOnApplyWindowInsetsListener((view, windowInsets) -> {
            android.graphics.Insets bars = windowInsets.getInsets(android.view.WindowInsets.Type.systemBars()
                | android.view.WindowInsets.Type.displayCutout());
            view.setPadding(bars.left, bars.top, bars.right, bars.bottom);
            return windowInsets;
        });
        message = new TextView(this);
        message.setPadding(24, 24, 24, 24);
        layout.addView(message);
        start = new Button(this);
        start.setText("Start host");
        start.setOnClickListener(view -> startForegroundService(new Intent(this, HostService.class)));
        layout.addView(start);
        log = new Button(this);
        log.setText("View host log");
        log.setOnClickListener(view -> {
            String text;
            try (RandomAccessFile file = new RandomAccessFile(new File(getFilesDir(), "host.log"), "r")) {
                file.seek(Math.max(0, file.length() - 16000));
                byte[] bytes = new byte[(int)(file.length() - file.getFilePointer())];
                file.readFully(bytes);
                text = new String(bytes, java.nio.charset.StandardCharsets.UTF_8);
            } catch (IOException error) { text = "No host log is available yet."; }
            TextView content = new TextView(this);
            content.setText(text);
            content.setTextIsSelectable(true);
            ScrollView scroll = new ScrollView(this);
            scroll.addView(content);
            new AlertDialog.Builder(this).setTitle("Host log").setView(scroll).setPositiveButton("Close", null).show();
        });
        layout.addView(log);
        web = new WebView(this);
        layout.addView(web, new LinearLayout.LayoutParams(-1, 0, 1));
        setContentView(layout);
        WebSettings settings = web.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        web.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri uri = request.getUrl();
                if (isLocal(uri)) return false;
                if (request.isForMainFrame() && "https".equals(uri.getScheme())) {
                    try { startActivity(new Intent(Intent.ACTION_VIEW, uri)); } catch (ActivityNotFoundException ignored) {}
                }
                return true;
            }
            @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                if (isLocal(request.getUrl())) return null;
                return new WebResourceResponse("text/plain", "UTF-8", 403, "Blocked", java.util.Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            @Override public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (upload != null) upload.onReceiveValue(null);
                upload = callback;
                Intent select = new Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE);
                select.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, params.getMode() == FileChooserParams.MODE_OPEN_MULTIPLE);
                startActivityForResult(select, 10);
                return true;
            }
        });
        web.setDownloadListener((url, agent, disposition, mime, length) -> {
            if (!isLocal(Uri.parse(url))) return;
            downloadUrl = url;
            Intent save = new Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(mime == null ? "application/octet-stream" : mime)
                .putExtra(Intent.EXTRA_TITLE, URLUtil.guessFileName(url, disposition, mime));
            startActivityForResult(save, 11);
        });
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 20);
        startForegroundService(new Intent(this, HostService.class));
    }
    private boolean isLocal(Uri uri) {
        if (origin == null) return false;
        Uri base = Uri.parse(origin);
        return "http".equals(uri.getScheme()) && base.getHost().equals(uri.getHost()) && base.getPort() == uri.getPort();
    }
    @Override protected void onStart() { super.onStart(); handler.post(poll); }
    @Override protected void onStop() { handler.removeCallbacks(poll); super.onStop(); }
    @Override public void onBackPressed() { if (web.canGoBack()) web.goBack(); else super.onBackPressed(); }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == 10 && upload != null) {
            java.util.ArrayList<Uri> chosen = new java.util.ArrayList<>();
            if (result == RESULT_OK && data != null) {
                if (data.getClipData() != null) {
                    for (int i = 0; i < data.getClipData().getItemCount(); i++) chosen.add(data.getClipData().getItemAt(i).getUri());
                } else if (data.getData() != null) chosen.add(data.getData());
            }
            chosen.removeIf(uri -> !"content".equals(uri.getScheme()));
            upload.onReceiveValue(chosen.isEmpty() ? null : chosen.toArray(new Uri[0]));
            upload = null;
        }
        if (request == 11 && result == RESULT_OK && data != null && data.getData() != null && downloadUrl != null) {
            String source = downloadUrl;
            Uri destination = data.getData();
            String cookie = android.webkit.CookieManager.getInstance().getCookie(source);
            downloadUrl = null;
            new Thread(() -> {
                HttpURLConnection connection = null;
                try {
                    if (!"content".equals(destination.getScheme()) || !isLocal(Uri.parse(source))) throw new IOException("Invalid download");
                    connection = (HttpURLConnection) new URL(source).openConnection();
                    connection.setInstanceFollowRedirects(false);
                    connection.setConnectTimeout(15000);
                    connection.setReadTimeout(60000);
                    if (cookie != null) connection.setRequestProperty("Cookie", cookie);
                    if (connection.getResponseCode() != 200) throw new IOException("HTTP " + connection.getResponseCode());
                    try (InputStream input = connection.getInputStream(); OutputStream output = getContentResolver().openOutputStream(destination)) {
                        if (output == null) throw new IOException("Cannot open destination");
                        input.transferTo(output);
                    }
                    runOnUiThread(() -> Toast.makeText(this, "Download saved", Toast.LENGTH_SHORT).show());
                } catch (Exception error) {
                    runOnUiThread(() -> Toast.makeText(this, "Download failed: " + error.getMessage(), Toast.LENGTH_LONG).show());
                } finally { if (connection != null) connection.disconnect(); }
            }, "mcpanel-download").start();
        }
    }
    @Override protected void onDestroy() {
        handler.removeCallbacks(poll);
        if (upload != null) upload.onReceiveValue(null);
        web.destroy();
        super.onDestroy();
    }
}
