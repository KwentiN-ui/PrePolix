#import "../../template.typ": *

== Animation <animation>

The animation button on the results toolbar opens the animation window.

#screenshot("animation.png", [Animation window])

#fields(
  [Type], [#ui("Scale factor") grows the displayed increment from undeformed to deformed; a mode
    swings between −1 and +1. #ui("Step increments") plays the increments of the step.],
  [Frames], [Number of frames of a scale factor animation (default 15).],
  [Frames per second], [Playback speed (default 15).],
  [Playback], [#ui("Once"), #ui("Loop") or #ui("Swing") (forth and back, default).],
  [Color scale], [#ui("Current frame") adapts the legend to every frame, #ui("All frames") keeps
    one range for the whole animation.],
)

The buttons step through the frames or play them; the slider selects a frame. Closing the window
returns to the increment shown before.

== Sound of the mode shapes <sound>

For a frequency step, the sound button opens #ui("Sound of the Mode Shapes"). It plays the
eigenfrequencies as tones, so you can hear how the structure would ring.

#screenshot("sound.png", [Sound of the mode shapes])

- Tick the modes to hear them together; #ui("Level") sets the volume of each mode. A click on a
  mode number shows this mode swinging in the 3D view.
- #ui("Make audible"): #ui("Automatic") shifts all frequencies by whole octaves into the audible
  range, keeping their intervals. #ui("Octaves") applies a fixed shift, #ui("Original") none.
- #ui("Sound"): #ui("Sustained") tones, or #ui("Struck") tones that decay like a bell, higher modes
  faster (#ui("Decay time")).
- #ui("Show superposed while playing") overlays the ticked modes in the 3D view, each swinging at
  its own frequency ratio, with #ui("Speed") for the lowest mode.
- #ui("Save as WAV...") writes the sound to a file.
