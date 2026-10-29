// omadeck's Omarchy bar widget: the Stream Deck's status, from `omadeckd status --follow`.
// Click to open the omadeck app. Written into ~/.config/omarchy/plugins/dev.omadeck/ by the
// app, which fills in @OMADECKD@ with the path of its own omadeckd.
import QtQuick
import Quickshell.Io
import qs.Ui

Item {
  id: root

  property var bar
  property string moduleName: "dev.omadeck"
  property var settings: ({})

  // "connected", "disconnected" or "stopped"; empty until omadeckd first answers.
  property string deckState: ""
  property string glyph: ""
  property string tip: ""

  visible: deckState !== ""
  implicitWidth: visible ? button.implicitWidth : 0
  implicitHeight: bar ? bar.barSize : 26

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.glyph
    tooltipText: root.tip
    dimmed: root.deckState !== "connected"
    onPressed: function() {
      if (root.bar) root.bar.run("uwsm-app -- dev.omadeck.Omadeck.desktop")
    }
  }

  Process {
    id: watcher
    command: ["@OMADECKD@", "status", "--follow"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        try {
          const d = JSON.parse(line)
          root.glyph = d.text || ""
          root.tip = d.tooltip || ""
          root.deckState = d["class"] || ""
        } catch (e) {}
      }
    }
    // It only stops if omadeckd is missing or crashed: try again later.
    onRunningChanged: if (!running) retry.start()
  }

  Timer {
    id: retry
    interval: 5000
    onTriggered: watcher.running = true
  }
}
