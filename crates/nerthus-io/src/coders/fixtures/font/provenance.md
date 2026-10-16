`tuffy.ttf` is ImageMagick 7.1.2-19's `PerlMagick/t/ttf/input.ttf`, the
Tuffy font by Thatcher Ulrich (https://tulrich.com/fonts/). Its embedded
copyright and license records state:

> This font, including hint instructions, has been donated to the Public
> Domain. Do whatever you want with it.

`tuffy.otf` contains the same outlines converted to CFF with FontTools
4.60.2's `FontBuilder` and `T2CharStringPen`, retaining the character map,
advances and kerning. This exercises the OpenType CFF reader separately
from TrueType outlines and their hint instructions.
