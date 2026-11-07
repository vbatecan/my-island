# Rebuilds original code-native capsule assets; no network or external artwork.
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
$assetDirectory=Join-Path $PSScriptRoot 'Assets'
$null=New-Item -ItemType Directory -Force $assetDirectory
foreach($asset in @(@('StoreLogo.png',50),@('Square44x44Logo.png',44),@('Square150x150Logo.png',150))) {
    $size=[int]$asset[1]
    $bitmap=[Drawing.Bitmap]::new($size,$size,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    $path=[Drawing.Drawing2D.GraphicsPath]::new()
    $outline=[Drawing.Pen]::new([Drawing.Color]::FromArgb(200,150,150,160),[single]($size/100.0))
    $black=[Drawing.SolidBrush]::new([Drawing.Color]::Black)
    $white=[Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(240,240,245))
    $gray=[Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(70,70,78))
    try {
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.SmoothingMode=[Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $left=[single]($size*0.09);$top=[single]($size*0.35);$width=[single]($size*0.82);$height=[single]($size*0.30)
        $path.AddArc($left,$top,$height,$height,90,180)
        $path.AddArc(($left+$width-$height),$top,$height,$height,270,180)
        $path.CloseFigure()
        $graphics.FillPath($black,$path);$graphics.DrawPath($outline,$path)
        $graphics.FillEllipse($gray,[single]($size*0.2),[single]($size*0.445),[single]($size*0.11),[single]($size*0.11))
        $graphics.FillEllipse($white,[single]($size*0.63),[single]($size*0.445),[single]($size*0.11),[single]($size*0.11))
        $bitmap.Save((Join-Path $assetDirectory $asset[0]),[Drawing.Imaging.ImageFormat]::Png)
    } finally {$gray.Dispose();$white.Dispose();$black.Dispose();$outline.Dispose();$path.Dispose();$graphics.Dispose();$bitmap.Dispose()}
}
