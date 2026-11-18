package de.owndraw;

import android.util.Log;

import com.google.android.gms.tasks.OnFailureListener;
import com.google.android.gms.tasks.OnSuccessListener;
import com.google.mlkit.common.MlKitException;
import com.google.mlkit.common.model.DownloadConditions;
import com.google.mlkit.common.model.RemoteModelManager;
import com.google.mlkit.vision.digitalink.common.RecognitionCandidate;
import com.google.mlkit.vision.digitalink.common.RecognitionResult;
import com.google.mlkit.vision.digitalink.recognition.DigitalInkRecognition;
import com.google.mlkit.vision.digitalink.recognition.DigitalInkRecognitionModel;
import com.google.mlkit.vision.digitalink.recognition.DigitalInkRecognitionModelIdentifier;
import com.google.mlkit.vision.digitalink.recognition.DigitalInkRecognizer;
import com.google.mlkit.vision.digitalink.recognition.DigitalInkRecognizerOptions;
import com.google.mlkit.vision.digitalink.recognition.Ink;
import com.google.mlkit.vision.digitalink.recognition.RecognitionContext;
import com.google.mlkit.vision.digitalink.recognition.WritingArea;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.List;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Handschrift-Erkennung mit Google ML Kit Digital Ink Recognition (Werkzeug „Umwandeln“ auf Android).
 *
 * Das Sprachmodell (Deutsch, „de“) lädt ML Kit nur auf Wunsch von Googles Servern (laden()); erkannt wird
 * danach offline auf dem Gerät. Rust ruft über OwnDrawActivity.hw(cmd, arg) auf (crates/owndraw/src/hw.rs):
 *   status            → {"zustand":"unbekannt|pruefe|fehlt|laedt|da|fehler","fehler":"…"}
 *   laden / loeschen  → Status wie oben
 *   erkennen(json)    → Auftragsnummer; json = {"nbest":5,"w":…,"h":…,"pre":"…","striche":[[x,y,x,y,…],…]}
 *   ergebnis(nr)      → "" (läuft) oder {"ok":true,"kandidaten":[{"text":"…","mlkit":1.5}],"ms":…}
 *                       bzw. {"ok":false,"fehler":"…"}; ML-Kit-Score: kleiner = besser, fehlt evtl.
 * Alle ML-Kit-Rückrufe laufen auf dem Haupt-Thread; die Zustände sind threadsicher abgelegt.
 * Erfolgs-Rückrufe sind bewusst OnSuccessListener&lt;Object&gt;: ein typisierter Rückruf bekäme eine
 * Bridge-Methode, deren MethodParameters (javac 21, ohne Namen) d8 8.2 (build-tools 34) abstürzen lassen
 * (mit JDK 21 trotzdem build-tools ≥ 35 nehmen, OwnDrawActivity hat solche Klassen auch).
 */
final class InkRecognizer {
    private static final String TAG = "OwnDraw";
    /** Sprache des Modells (BCP 47). */
    static final String SPRACHE = "de";

    private static volatile String zustand = "unbekannt";
    private static volatile String fehler = "";
    private static DigitalInkRecognitionModel model;
    private static DigitalInkRecognizer recognizer;
    private static final AtomicInteger nextJob = new AtomicInteger(1);
    private static final ConcurrentHashMap<Integer, String> results = new ConcurrentHashMap<>();
    private static final ConcurrentHashMap<Integer, Boolean> running = new ConcurrentHashMap<>();

    private InkRecognizer() {}

    static String call(String cmd, String arg) {
        try {
            switch (cmd) {
                case "status":
                    if (zustand.equals("unbekannt")) pruefen();
                    return status();
                case "laden":
                    laden();
                    return status();
                case "loeschen":
                    loeschen();
                    return status();
                case "erkennen":
                    return Integer.toString(erkennen(arg));
                case "ergebnis":
                    return ergebnis(Integer.parseInt(arg));
                default:
                    return "";
            }
        } catch (Exception e) {
            Log.w(TAG, "InkRecognizer." + cmd, e);
            return fehlerJson(e.toString());
        }
    }

    private static synchronized DigitalInkRecognitionModel model() throws MlKitException {
        if (model == null) {
            DigitalInkRecognitionModelIdentifier id = DigitalInkRecognitionModelIdentifier.fromLanguageTag(SPRACHE);
            if (id == null) throw new MlKitException("Kein ML-Kit-Modell für " + SPRACHE, MlKitException.NOT_FOUND);
            model = DigitalInkRecognitionModel.builder(id).build();
        }
        return model;
    }

    private static String status() {
        try {
            JSONObject o = new JSONObject();
            o.put("zustand", zustand);
            o.put("fehler", fehler);
            o.put("sprache", SPRACHE);
            return o.toString();
        } catch (JSONException e) {
            return "{\"zustand\":\"fehler\"}";
        }
    }

    private static void setze(String z, String f) {
        zustand = z;
        fehler = f == null ? "" : f;
    }

    private static void pruefen() throws MlKitException {
        setze("pruefe", "");
        RemoteModelManager.getInstance().isModelDownloaded(model())
            .addOnSuccessListener(new OnSuccessListener<Object>() {
                @Override
                public void onSuccess(Object da) {
                    if (zustand.equals("pruefe")) setze(Boolean.TRUE.equals(da) ? "da" : "fehlt", "");
                }
            })
            .addOnFailureListener(new OnFailureListener() {
                @Override
                public void onFailure(Exception e) {
                    if (zustand.equals("pruefe")) setze("fehler", e.getMessage());
                }
            });
    }

    private static void laden() throws MlKitException {
        if (zustand.equals("laedt") || zustand.equals("da")) return;
        setze("laedt", "");
        RemoteModelManager.getInstance().download(model(), new DownloadConditions.Builder().build())
            .addOnSuccessListener(new OnSuccessListener<Object>() {
                @Override
                public void onSuccess(Object v) {
                    setze("da", "");
                }
            })
            .addOnFailureListener(new OnFailureListener() {
                @Override
                public void onFailure(Exception e) {
                    Log.w(TAG, "ML-Kit-Modell laden", e);
                    setze("fehler", e.getMessage());
                }
            });
    }

    private static void loeschen() throws MlKitException {
        synchronized (InkRecognizer.class) {
            if (recognizer != null) {
                recognizer.close();
                recognizer = null;
            }
        }
        setze("pruefe", "");
        RemoteModelManager.getInstance().deleteDownloadedModel(model())
            .addOnSuccessListener(new OnSuccessListener<Object>() {
                @Override
                public void onSuccess(Object v) {
                    setze("fehlt", "");
                }
            })
            .addOnFailureListener(new OnFailureListener() {
                @Override
                public void onFailure(Exception e) {
                    setze("fehler", e.getMessage());
                }
            });
    }

    private static synchronized DigitalInkRecognizer recognizer() throws MlKitException {
        if (recognizer == null) {
            recognizer = DigitalInkRecognition.getClient(DigitalInkRecognizerOptions.builder(model()).build());
        }
        return recognizer;
    }

    private static int erkennen(String arg) throws JSONException, MlKitException {
        JSONObject a = new JSONObject(arg);
        final int nbest = Math.max(1, a.optInt("nbest", 5));
        JSONArray striche = a.getJSONArray("striche");
        Ink.Builder ink = Ink.builder();
        for (int i = 0; i < striche.length(); i++) {
            JSONArray p = striche.getJSONArray(i);
            if (p.length() < 2) continue;
            Ink.Stroke.Builder s = Ink.Stroke.builder();
            for (int j = 0; j + 1 < p.length(); j += 2) {
                // Ohne Zeitstempel: OwnDraw speichert keine Zeiten je Punkt (ML Kit erlaubt das).
                s.addPoint(Ink.Point.create((float) p.getDouble(j), (float) p.getDouble(j + 1)));
            }
            ink.addStroke(s.build());
        }
        RecognitionContext.Builder ctx = RecognitionContext.builder().setPreContext(a.optString("pre", ""));
        float w = (float) a.optDouble("w", 0), h = (float) a.optDouble("h", 0);
        if (w > 0 && h > 0) ctx.setWritingArea(new WritingArea(w, h));
        final int job = nextJob.getAndIncrement();
        final long t0 = System.currentTimeMillis();
        running.put(job, Boolean.TRUE);
        recognizer().recognize(ink.build(), ctx.build())
            .addOnSuccessListener(new OnSuccessListener<Object>() {
                @Override
                public void onSuccess(Object res) {
                    try {
                        RecognitionResult r = (RecognitionResult) res;
                        JSONArray k = new JSONArray();
                        List<RecognitionCandidate> cs = r.getCandidates();
                        for (int i = 0; i < cs.size() && i < nbest; i++) {
                            JSONObject c = new JSONObject();
                            c.put("text", cs.get(i).getText());
                            Float sc = cs.get(i).getScore();
                            if (sc != null) c.put("mlkit", sc.doubleValue());
                            k.put(c);
                        }
                        JSONObject o = new JSONObject();
                        o.put("ok", true);
                        o.put("kandidaten", k);
                        o.put("ms", System.currentTimeMillis() - t0);
                        fertig(job, o.toString());
                    } catch (JSONException e) {
                        fertig(job, fehlerJson(e.toString()));
                    }
                }
            })
            .addOnFailureListener(new OnFailureListener() {
                @Override
                public void onFailure(Exception e) {
                    Log.w(TAG, "ML-Kit-Erkennung", e);
                    fertig(job, fehlerJson(e.getMessage()));
                }
            });
        return job;
    }

    private static void fertig(int job, String json) {
        results.put(job, json);
        running.remove(job);
    }

    private static String ergebnis(int job) {
        String r = results.remove(job);
        if (r != null) return r;
        return running.containsKey(job) ? "" : fehlerJson("Unbekannter Auftrag");
    }

    private static String fehlerJson(String msg) {
        try {
            JSONObject o = new JSONObject();
            o.put("ok", false);
            o.put("fehler", msg == null ? "Erkennung fehlgeschlagen" : msg);
            return o.toString();
        } catch (JSONException e) {
            return "{\"ok\":false}";
        }
    }
}
