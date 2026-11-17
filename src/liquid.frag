#version 330 core
uniform int optics; // Acrylic or Glass
uniform sampler2D backdrop;
uniform vec2 canvas;
uniform vec2 target_size;
uniform vec4 shape; // total width, height, radius, shoulder
uniform float sample_y;
out vec4 color;
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
    vec2 logical=uv*canvas;
    vec2 p=logical-vec2((canvas.x-shape.x)*.5,0);
    // Most of the persistent canvas is transparent, especially when compact.
    if(p.x<0. || p.y<0. || p.x>shape.x || p.y>shape.y) {color=vec4(0);return;}
    vec3 dn=distance_normal(p);
    float coverage=clamp(.5-dn.x*target_size.x/canvas.x,0.,1.);
    if(coverage==0.) { color=vec4(0); return; }
    // Capture stays anchored to the settled window position while the notch
    // slides. Read from the actual desktop position of each rendered pixel.
    vec2 backdrop_uv=uv+vec2(0.,sample_y/canvas.y);
    // Acrylic receives a prepared blurred backdrop once per capture. Only the
    // contour mask changes during the morph; no CPU rasterization or readback.
    if(optics==1) { color=vec4(texture(backdrop,backdrop_uv).rgb,coverage); return; }
    float depth=max(-dn.x,0.);
    // Fade the bevel lighting to zero at the upward-facing edge.
    float rim=clamp(1.+dn.z,0.,1.);
    float bezel=min(12.,shape.y*.46);
    const float f0=(.45/2.45)*(.45/2.45);
    float reflection=f0*.55+pow(.61,18.)*.16;
    vec2 refracted=backdrop_uv;
    if(depth<bezel) {
        float t=clamp(depth/bezel,.0001,1.);
        float arc=sqrt(1.-(1.-t)*(1.-t));
        float slope=min(8.,8.*(1.-t)/(bezel*arc));
        vec3 normal=normalize(vec3(dn.yz*slope,1.));
        vec3 ray=refract(vec3(0.,0.,-1.),normal,1./1.45);
        refracted+=ray.xy*(12./max(-ray.z,.05))/canvas;
        float fresnel=f0+(1.-f0)*pow(1.-normal.z,5.);
        float light=max(dot(normal,vec3(-.45,-.65,.61)),0.);
        reflection=mix(reflection,fresnel*.55+pow(light,18.)*.16,rim);
    }
    // Slint 1.18's borrowed-texture importer expects straight RGBA and
    // premultiplies at sampling. Coverage is applied exactly once there.
    color=vec4(mix(texture(backdrop,refracted).rgb,vec3(1.),reflection*coverage),coverage);
}
