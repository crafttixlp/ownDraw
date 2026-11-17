package de.owndraw;

import android.app.NativeActivity;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipDescription;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.util.Log;
import android.view.DragAndDropPermissions;
import android.view.DragEvent;
import android.view.View;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.ArrayList;

/**
 * Hülle um die NativeActivity von OwnDraw (Rust/egui in libowndraw.so).
 *
 * Leitet an die native Seite weiter, was nur Java mitbekommt: Ergebnisse der SAF-Dialoge
 * (onActivityResult), onPause, Start-/neue Intents („Öffnen mit“, „Teilen“) und Drag &amp; Drop.
 * Dazu Hilfsmethoden, die Rust per JNI aufruft: Zwischenablage (clipSet/clipGet) und openUrl.
 */
public class OwnDrawActivity extends NativeActivity {
    private static final String TAG = "OwnDraw";

    static {
        // NativeActivity lädt die Bibliothek selbst per dlopen; für die JNI-Methoden unten muss sie
        // zusätzlich über den ClassLoader registriert sein (gleiche Datei, gleicher Handle).
        System.loadLibrary("owndraw");
    }

    private native void nativeOnPicked(int tag, String uri);
    private native void nativeOnPause();
    private native void nativeOnOpen(String uri);
    private native void nativeOnDrop(String uri, String text, String mime, float x, float y);
    private native void nativeOnDragHover(boolean hover);

    /** Rechte an fallen gelassenen URIs; gelten, bis sie freigegeben werden oder die Activity endet. */
    private final ArrayList<DragAndDropPermissions> dropPermissions = new ArrayList<>();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        getWindow().getDecorView().setOnDragListener(new View.OnDragListener() {
            @Override
            public boolean onDrag(View v, DragEvent e) {
                return handleDrag(e);
            }
        });
        // Nach Neuerstellung (Drehen o. Ä.) den Start-Intent nicht erneut öffnen.
        if (savedInstanceState == null) {
            handleIntent(getIntent());
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        handleIntent(intent);
    }

    @Override
    protected void onPause() {
        // Vor super.onPause(): die App speichert, solange sie noch läuft (wartet höchstens kurz).
        nativeOnPause();
        super.onPause();
    }

    @Override
    protected void onDestroy() {
        for (DragAndDropPermissions p : dropPermissions) {
            p.release();
        }
        dropPermissions.clear();
        super.onDestroy();
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        Uri uri = (resultCode == RESULT_OK && data != null) ? data.getData() : null;
        if (uri != null) {
            persist(uri, data.getFlags());
        }
        nativeOnPicked(requestCode, uri == null ? null : uri.toString());
    }

    /** Dauerhafte Rechte (übersteht Neustart), soweit der Anbieter sie gewährt. */
    private void persist(Uri uri, int flags) {
        int f = flags & (Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        if (f == 0 || (flags & Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) == 0) {
            return;
        }
        try {
            getContentResolver().takePersistableUriPermission(uri, f);
        } catch (SecurityException e) {
            Log.w(TAG, "takePersistableUriPermission: " + e);
        }
    }

    /** VIEW („Öffnen mit“) und SEND („Teilen an OwnDraw“). */
    private void handleIntent(Intent intent) {
        if (intent == null) {
            return;
        }
        String action = intent.getAction();
        Uri uri = null;
        if (Intent.ACTION_VIEW.equals(action)) {
            uri = intent.getData();
        } else if (Intent.ACTION_SEND.equals(action)) {
            uri = streamExtra(intent);
        }
        if (uri != null) {
            persist(uri, intent.getFlags());
            nativeOnOpen(uri.toString());
        }
    }

    @SuppressWarnings("deprecation")
    private static Uri streamExtra(Intent intent) {
        Object o = intent.getParcelableExtra(Intent.EXTRA_STREAM);
        if (o instanceof Uri) {
            return (Uri) o;
        }
        ClipData c = intent.getClipData();
        if (c != null && c.getItemCount() > 0) {
            return c.getItemAt(0).getUri();
        }
        return null;
    }

    // ------------------------------------------------------------- Drag & Drop

    private boolean handleDrag(DragEvent e) {
        switch (e.getAction()) {
            case DragEvent.ACTION_DRAG_STARTED:
                return true;
            case DragEvent.ACTION_DRAG_ENTERED:
                nativeOnDragHover(true);
                return true;
            case DragEvent.ACTION_DRAG_EXITED:
            case DragEvent.ACTION_DRAG_ENDED:
                nativeOnDragHover(false);
                return true;
            case DragEvent.ACTION_DROP:
                nativeOnDragHover(false);
                DragAndDropPermissions p = requestDragAndDropPermissions(e);
                if (p != null) {
                    dropPermissions.add(p);
                }
                ClipData clip = e.getClipData();
                if (clip == null) {
                    return false;
                }
                ClipDescription desc = e.getClipDescription();
                String mime = (desc != null && desc.getMimeTypeCount() > 0) ? desc.getMimeType(0) : null;
                for (int i = 0; i < clip.getItemCount(); i++) {
                    ClipData.Item it = clip.getItemAt(i);
                    Uri u = it.getUri();
                    CharSequence t = it.getText();
                    String m = mime;
                    if (u != null) {
                        String real = getContentResolver().getType(u);
                        if (real != null) {
                            m = real;
                        }
                    }
                    nativeOnDrop(u == null ? null : u.toString(), t == null ? null : t.toString(), m, e.getX(), e.getY());
                }
                return true;
            default:
                return true;
        }
    }

    // ------------------------------------------------------------- Aufrufe aus Rust (JNI)

    /**
     * Setzt die Zwischenablage. {@code pngPath}: Bild im Cache-Ordner (wird per FileShareProvider als
     * content://-URI angeboten), {@code text}: Text bzw. interne Items mit Präfix "owndraw-items:".
     * Beides darf null sein; Bild steht vorn, damit andere Apps es als erstes Element finden.
     */
    public void clipSet(final String text, final String pngPath) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                try {
                    ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
                    ClipData clip = null;
                    Uri img = pngPath == null ? null : FileShareProvider.uriFor(OwnDrawActivity.this, new File(pngPath));
                    if (img != null) {
                        clip = ClipData.newUri(getContentResolver(), "OwnDraw", img);
                        if (text != null) {
                            clip.addItem(getContentResolver(), new ClipData.Item(text));
                        }
                    } else if (text != null) {
                        clip = ClipData.newPlainText("OwnDraw", text);
                    }
                    if (clip != null) {
                        cm.setPrimaryClip(clip);
                    }
                } catch (RuntimeException e) {
                    Log.w(TAG, "clipSet: " + e);
                }
            }
        });
    }

    /** Inhalt der Zwischenablage: "t" + Text bzw. "u" + MIME + "\n" + URI je Eintrag. */
    public String[] clipGet() {
        ArrayList<String> out = new ArrayList<>();
        try {
            ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
            ClipData clip = cm.getPrimaryClip();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    ClipData.Item it = clip.getItemAt(i);
                    Uri u = it.getUri();
                    if (u != null) {
                        String m = getContentResolver().getType(u);
                        out.add("u" + (m == null ? "" : m) + "\n" + u);
                    }
                    CharSequence t = it.getText();
                    if (t != null) {
                        out.add("t" + t);
                    }
                }
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "clipGet: " + e);
        }
        return out.toArray(new String[0]);
    }

    /** Öffnet eine URL im Browser bzw. eine Datei aus dem App-Speicher (über FileShareProvider). */
    public void openUrl(String url) {
        try {
            Intent i = new Intent(Intent.ACTION_VIEW);
            String path = url.startsWith("file://") ? Uri.parse(url).getPath() : (url.startsWith("/") ? url : null);
            if (path != null) {
                File f = new File(path);
                Uri u = FileShareProvider.uriFor(this, f);
                if (u == null) {
                    Log.w(TAG, "openUrl: Datei liegt nicht im App-Speicher: " + path);
                    return;
                }
                String ext = MimeTypeMap.getFileExtensionFromUrl(f.getName());
                String mime = ext == null ? null : MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext.toLowerCase());
                i.setDataAndType(u, mime == null ? "*/*" : mime);
                i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            } else {
                i.setData(Uri.parse(url));
            }
            i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(i);
        } catch (ActivityNotFoundException | SecurityException e) {
            Log.w(TAG, "openUrl: " + e);
        }
    }

    /** Teilt eine Datei aus dem App-Speicher über das Teilen-Menü von Android. */
    public void shareFile(String path, String mime) {
        try {
            Uri u = FileShareProvider.uriFor(this, new File(path));
            if (u == null) {
                Log.w(TAG, "shareFile: Datei liegt nicht im App-Speicher: " + path);
                return;
            }
            Intent i = new Intent(Intent.ACTION_SEND);
            i.setType(mime);
            i.putExtra(Intent.EXTRA_STREAM, u);
            i.setClipData(ClipData.newRawUri("", u));
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            Intent c = Intent.createChooser(i, "Teilen");
            c.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(c);
        } catch (ActivityNotFoundException | SecurityException e) {
            Log.w(TAG, "shareFile: " + e);
        }
    }

    /** Druckt ein PDF aus dem App-Speicher über den Druckdienst von Android. */
    public void printPdf(final String path, final String name) {
        // ohne Lambda: der Build nutzt javac mit android.jar ohne LambdaMetafactory
        runOnUiThread(new Runnable() {
          @Override
          public void run() {
            try {
                android.print.PrintManager pm = (android.print.PrintManager) getSystemService(Context.PRINT_SERVICE);
                pm.print(name, new android.print.PrintDocumentAdapter() {
                    @Override
                    public void onLayout(android.print.PrintAttributes o, android.print.PrintAttributes n, android.os.CancellationSignal cs,
                            LayoutResultCallback cb, Bundle extras) {
                        if (cs.isCanceled()) {
                            cb.onLayoutCancelled();
                            return;
                        }
                        cb.onLayoutFinished(new android.print.PrintDocumentInfo.Builder(name)
                                .setContentType(android.print.PrintDocumentInfo.CONTENT_TYPE_DOCUMENT).build(), true);
                    }

                    @Override
                    public void onWrite(android.print.PageRange[] pages, android.os.ParcelFileDescriptor dest, android.os.CancellationSignal cs,
                            WriteResultCallback cb) {
                        try (InputStream in = new java.io.FileInputStream(path);
                             OutputStream out = new FileOutputStream(dest.getFileDescriptor())) {
                            byte[] buf = new byte[65536];
                            int n;
                            while ((n = in.read(buf)) > 0) {
                                out.write(buf, 0, n);
                            }
                            cb.onWriteFinished(new android.print.PageRange[] { android.print.PageRange.ALL_PAGES });
                        } catch (IOException e) {
                            cb.onWriteFailed(e.toString());
                        }
                    }
                }, null);
            } catch (RuntimeException e) {
                Log.w(TAG, "printPdf: " + e);
            }
          }
        });
    }

    // ---- Update-Dienst (WP11) ----

    private static volatile boolean updCancel;

    /** Bricht einen laufenden httpDownload ab. */
    public void updateCancel() {
        updCancel = true;
    }

    private static HttpURLConnection openHttps(String url) throws IOException {
        if (!url.startsWith("https://")) {
            throw new IOException("Nur HTTPS erlaubt");
        }
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(20000);
        c.setReadTimeout(30000);
        c.setRequestProperty("User-Agent", "OwnDraw-Updater");
        c.setRequestProperty("Accept", "application/vnd.github+json");
        int code = c.getResponseCode();
        if (code < 200 || code >= 300) {
            throw new IOException("http status: " + code);
        }
        return c;
    }

    /** Textantwort (GitHub-API, Signaturdatei); höchstens 8 MB. */
    public String httpGet(String url) throws IOException {
        HttpURLConnection c = openHttps(url);
        try (InputStream in = c.getInputStream()) {
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            byte[] buf = new byte[16384];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
                if (out.size() > 8 * 1024 * 1024) {
                    throw new IOException("Antwort zu groß");
                }
            }
            return out.toString("UTF-8");
        } finally {
            c.disconnect();
        }
    }

    /** Lädt nach dest; Rückgabe: Bytes. Bricht bei updateCancel() mit IOException ab. */
    public long httpDownload(String url, String dest) throws IOException {
        updCancel = false;
        HttpURLConnection c = openHttps(url);
        long total = 0;
        try (InputStream in = c.getInputStream(); OutputStream out = new FileOutputStream(dest)) {
            byte[] buf = new byte[65536];
            int n;
            while ((n = in.read(buf)) > 0) {
                if (updCancel) {
                    throw new IOException("abgebrochen");
                }
                out.write(buf, 0, n);
                total += n;
            }
        } finally {
            c.disconnect();
        }
        long expected = c.getContentLengthLong();
        if (expected >= 0 && expected != total) {
            throw new IOException("Download unvollständig");
        }
        return total;
    }

    /**
     * Übergibt die geprüfte APK an den System-Installer. Fehlt das Recht „Unbekannte Apps installieren“,
     * öffnet sich stattdessen die passende Systemeinstellung.
     */
    public String installApk(String path) throws IOException {
        if (Build.VERSION.SDK_INT >= 26 && !getPackageManager().canRequestPackageInstalls()) {
            Intent s = new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + getPackageName()));
            s.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            startActivity(s);
            return "Erlauben Sie OwnDraw zuerst „Unbekannte Apps installieren“ und tippen Sie danach erneut auf „Jetzt aktualisieren“.";
        }
        Uri u = FileShareProvider.uriFor(this, new File(path));
        if (u == null) {
            throw new IOException("APK liegt nicht im App-Speicher");
        }
        Intent i = new Intent(Intent.ACTION_VIEW);
        i.setDataAndType(u, "application/vnd.android.package-archive");
        i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
        startActivity(i);
        return "System-Installer geöffnet.";
    }
}
