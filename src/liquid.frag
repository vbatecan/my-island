#version 330 core
uniform sampler2D backdrop;
uniform sampler2D blurred;
uniform vec2 canvas;
uniform vec2 target_size;
uniform vec4 shape; // total width, height, radius, shoulder
uniform vec4 edges[16]; // two vec4s per shared line/arc
uniform float tint;
uniform float support; // +1 for dark text, -1 for light text
uniform int pass;
out vec4 color;
vec3 linearize(vec3 c) { return mix(c / 12.92, pow((c + .055) / 1.055, vec3(2.4)), step(vec3(.04045), c)); }
vec3 encode(vec3 c) { return mix(c * 12.92, 1.055 * pow(max(c, vec3(0)), vec3(1.0/2.4)) - .055, step(vec3(.0031308), c)); }
bool inside(vec2 p) {
    float w=shape.x, h=shape.y, r=shape.z, s=shape.w;
    if(p.x<0. || p.y<0. || p.x>w || p.y>h) return false;
    if(s==0.) { vec2 q=max(max(vec2(r)-p,p-vec2(w-r,h-r)),vec2(0)); return dot(q,q)<=r*r; }
    if(p.x<s || p.x>w-s) { float dx=min(p.x,w-p.x); return p.y<=s && dx*dx+(p.y-s)*(p.y-s)>=s*s; }
    vec2 q=vec2(max(max(s+r-p.x,p.x-(w-s-r)),0.),max(p.y-(h-r),0.));
    return dot(q,q)<=r*r;
}
vec3 distance_normal(vec2 p) {
    float w=shape.x,h=shape.y,r=shape.z,s=shape.w;
    if(s==0.) {
        vec2 v=p-vec2(w,h)*.5;
        vec2 q=abs(v)-vec2(w,h)*.5+r;
        vec2 o=max(q,0.); float len=length(o);
        vec2 n=len>1e-6?o/len*sign(v):(q.x>q.y?vec2(sign(v.x),0):vec2(0,sign(v.y)));
        return vec3(len+min(max(q.x,q.y),0.)-r,n);
    }
    // Exact notch shoulders and lower corners; no per-pixel arc enumeration.
    float depth=p.y; vec2 n=vec2(0,-1);
    float side=min(p.x,w-p.x); float direction=p.x<w*.5?-1.:1.;
    if(p.y<s) {
        vec2 v=vec2(side,p.y-s); float len=length(v);
        float d=len-s;
        if(d<depth) {depth=d;n=vec2(direction*v.x,-v.y)/max(len,1e-6);}
    } else {
        float d=side-s; vec2 normal=vec2(direction,0);
        if(p.y>h-r && side<s+r) {
            vec2 v=vec2(side-s-r,p.y-h+r); float len=length(v);
            d=r-len; normal=vec2(-direction*v.x,v.y)/max(len,1e-6);
        }
        if(d<depth) {depth=d;n=normal;}
        if(h-p.y<depth) {depth=h-p.y;n=vec2(0,1);}
    }
    return vec3(-depth,n);
}
void main() {
    // Texture row zero corresponds to the top of the canvas throughout this pipeline.
    vec2 uv=gl_FragCoord.xy/target_size;
    if(pass==1 || pass==2) {
        vec2 axis=(pass==1?vec2(1,0):vec2(0,1))*1.5/canvas;
        vec3 c=vec3(0);
        for(int i=-2;i<=2;i++) {
            float weight=i==0?.375:(abs(i)==1?.25:.0625);
            c+=weight*linearize(texture(backdrop,uv+float(i)*axis).rgb);
        }
        color=vec4(encode(c),1); return;
    }
    vec2 logical=uv*canvas;
    vec2 p=logical-vec2((canvas.x-shape.x)*.5,0);
    // Most of the persistent canvas is transparent, especially when compact.
    if(p.x<0. || p.y<0. || p.x>shape.x || p.y>shape.y) {color=vec4(0);return;}
    vec3 dn=distance_normal(p);
    float coverage=clamp(.5-dn.x*target_size.x/canvas.x,0.,1.);
    if(coverage==0.) { color=vec4(0); return; }
    float depth=max(-dn.x,0.);
    // Convex superellipse cross section: finite slope and a smooth flat centre.
    float t=clamp(1.-depth/8.,0.,.999);
    float slope=min(8.,pow(t,3.)/pow(max(1.-pow(t,4.),.001),.75));
    vec3 n=normalize(vec3(dn.yz*slope,1));
    vec3 ray=refract(vec3(0,0,-1),n,1./1.5);
    vec2 displacement=ray.xy*(6./max(-ray.z,.15));
    float len=length(displacement);
    displacement*=min(1.,6./max(len,1e-6));
    vec2 refracted=uv+displacement/canvas;
    vec3 c=mix(linearize(texture(backdrop,refracted).rgb),linearize(texture(blurred,refracted).rgb),.35);
    float fresnel=.04+.96*pow(1.-n.z,5.);
    float edge=1.-smoothstep(0.,8.,depth);
    float directional=max(dot(n,normalize(vec3(-.45,-.65,1))),0.);
    // Reflection stays on the curved bezel; the flat centre retains its hue.
    float glint=exp(-depth/1.1)*pow(directional,4.);
    c=mix(c,vec3(.85,.91,1),edge*(fresnel*.25+glint*.16));
    c*=1.-.075*exp(-pow((depth-2.5)/1.4,2.));
    c*=1.-.10*tint;
    // Feathered support is restricted to the content inset, never the entire panel.
    vec2 inset=min(p-vec2(shape.w+14.,8.),vec2(shape.x-shape.w-14.,shape.y-8.)-p);
    float local=smoothstep(0.,12.,min(inset.x,inset.y));
    float lum=dot(c,vec3(.2126,.7152,.0722));
    float need=support>0.?1.-smoothstep(.20,.55,lum):smoothstep(.20,.55,lum);
    // Never bleach the backdrop to support dark text.
    c*=1.-(support<0.?.15*local*need:0.);
    // Slint 1.18's borrowed-texture importer expects straight RGBA and
    // premultiplies at sampling. Coverage is applied exactly once there.
    color=vec4(encode(clamp(c,0.,1.)),coverage);
}
