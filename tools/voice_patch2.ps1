$ErrorActionPreference = 'Stop'
$p = 'C:\Users\Admin\Projects\FreeViewer\src\main.rs'
$enc = New-Object System.Text.UTF8Encoding($false)
$s = [System.IO.File]::ReadAllText($p, $enc)

$pat = '(?<ind>[ ]+)ui\.separator\(\);\r?\n[ ]+let mut want_pick = false;'
$m = [regex]::Matches($s, $pat)
Write-Output ("Treffer: " + $m.Count)
if ($m.Count -ne 1) { Write-Output 'ABBRUCH'; exit 1 }

$block = @'
                ui.separator();
                {
                    let v = self.shared.voice.clone();
                    let mic = v.mic.load(Ordering::Relaxed);
                    if ui
                        .selectable_label(mic, if mic { "Mikro an" } else { "Mikro aus" })
                        .on_hover_text("Sprache zum anderen Rechner senden")
                        .clicked()
                    {
                        v.mic.store(!mic, Ordering::Relaxed);
                    }
                    let spk = v.speaker.load(Ordering::Relaxed);
                    if ui
                        .selectable_label(spk, if spk { "Ton an" } else { "Ton aus" })
                        .on_hover_text("Sprache der anderen Seite abspielen")
                        .clicked()
                    {
                        v.speaker.store(!spk, Ordering::Relaxed);
                    }
                    if mic || v.got.load(Ordering::Relaxed) > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "Pegel {} / {}",
                                v.level_out.load(Ordering::Relaxed),
                                v.level_in.load(Ordering::Relaxed)
                            ))
                            .weak()
                            .size(11.0),
                        );
                    }
                }

'@
$s = $s.Remove($m[0].Index, 0).Insert($m[0].Index, $block)
[System.IO.File]::WriteAllText($p, $s, $enc)
Write-Output 'Sitzungsleiste erweitert'
