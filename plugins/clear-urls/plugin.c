// Workshop Clear URLs. Rules based on Sunbird Clear URLs by amaanq, MIT.
// No WASI or libc imports. Input/output are UTF-8 in a bounded shared buffer.
static unsigned char buffer[65536];
static int eq(const unsigned char *s,int n,const char *t){int i=0;while(t[i]){if(i>=n||s[i]!=(unsigned char)t[i])return 0;i++;}return i==n;}
static int domain(const unsigned char *host,int n,const char *name){int m=0;while(name[m])m++;return n>=m&&eq(host+n-m,m,name)&&(n==m||host[n-m-1]=='.');}
static int hex(int c){if(c>='0'&&c<='9')return c-'0';if(c>='a'&&c<='f')return c-'a'+10;if(c>='A'&&c<='F')return c-'A'+10;return -1;}
static int tracker(const unsigned char *s,int n,const unsigned char *host,int hn){
 unsigned char key[128];int k=0;
 for(int i=0;i<n;i++){int c=s[i];if(c=='%'){if(i+2>=n)return 0;int a=hex(s[i+1]),b=hex(s[i+2]);if(a<0||b<0)return 0;c=a*16+b;i+=2;}if(k==128)return 0;key[k++]=c;}
 if(k>=4&&key[0]=='u'&&key[1]=='t'&&key[2]=='m'&&key[3]=='_')return 1;
 const char *names[]={"ref_src","ref_url","fbclid","gclid","dclid","gbraid","wbraid","msclkid","yclid","twclid","ttclid","igshid","igsh","mc_eid","mc_cid","_hsenc","_hsmi","mkt_tok","oly_anon_id","oly_enc_id","vero_id","wickedid","__s"};
 for(unsigned i=0;i<sizeof(names)/sizeof(names[0]);i++)if(eq(key,k,names[i]))return 1;
 int social=domain(host,hn,"twitter.com")||domain(host,hn,"x.com")||domain(host,hn,"tiktok.com");
 int youtube=domain(host,hn,"youtube.com")||domain(host,hn,"youtu.be");
 if(eq(key,k,"s")||eq(key,k,"t"))return social;
 if(eq(key,k,"feature")||eq(key,k,"pp"))return youtube;
 if(eq(key,k,"si"))return youtube||domain(host,hn,"spotify.com")||domain(host,hn,"soundcloud.com");
 return 0;
}
static int lower(int c){return c>='A'&&c<='Z'?c+32:c;}
static int scheme(int i,int n){const char *p="http";for(int j=0;j<4;j++)if(i+j>=n||lower(buffer[i+j])!=p[j])return 0;int j=i+4;if(j<n&&lower(buffer[j])=='s')j++;return j+2<n&&buffer[j]==':'&&buffer[j+1]=='/'&&buffer[j+2]=='/'?j+3:0;}
static int stop(int c){return c<=32||c=='<'||c=='>'||c=='`'||c=='|'||c=='('||c==')';}
static int suffix(int c){return c=='.'||c==','||c=='!'||c=='?'||c==';'||c==':'||c==']';}
static int clean(int start,int end,int authority,int out){
 int query=-1,hash=end;for(int i=start;i<end;i++){if(buffer[i]=='#'){hash=i;break;}if(buffer[i]=='?'&&query<0)query=i;}
 if(query<0){for(int i=start;i<end;i++)buffer[out++]=buffer[i];return out;}
 int he=authority;while(he<query&&buffer[he]!='/'&&buffer[he]!='?'&&buffer[he]!='#')he++;
 int hs=authority;for(int i=authority;i<he;i++)if(buffer[i]=='@')hs=i+1;
 for(int i=hs;i<he;i++)if(buffer[i]==':'){he=i;break;}
 if(he>hs&&buffer[he-1]=='.')he--;
 unsigned char host[256];int hn=he-hs;if(hn>255)hn=0;for(int i=0;i<hn;i++)host[i]=lower(buffer[hs+i]);
 for(int i=start;i<query;i++)buffer[out++]=buffer[i];
 int kept=0;for(int i=query+1;i<hash;){int endpair=i;while(endpair<hash&&buffer[endpair]!='&')endpair++;int keyend=i;while(keyend<endpair&&buffer[keyend]!='=')keyend++;
 if(endpair>i&&!tracker(buffer+i,keyend-i,host,hn)){buffer[out++]=kept++?'&':'?';for(int j=i;j<endpair;j++)buffer[out++]=buffer[j];}i=endpair+1;}
 for(int i=hash;i<end;i++)buffer[out++]=buffer[i];return out;
}
__attribute__((export_name("workshop_abi_version"))) int version(void){return 1;}
__attribute__((export_name("workshop_start"))) void start(void){}
__attribute__((export_name("workshop_buffer"))) unsigned char *get_buffer(void){return buffer;}
__attribute__((export_name("workshop_transform"))) int transform(int length){
 if(length<0||length>65536)return -1;
 int out=0;for(int i=0;i<length;){int authority=scheme(i,length);if(!authority){buffer[out++]=buffer[i++];continue;}
 int end=authority;while(end<length){if(buffer[end]=='('){int close=end+1;while(close<length&&!stop(buffer[close]))close++;if(close<length&&buffer[close]==')'){end=close+1;continue;}}if(stop(buffer[end]))break;end++;}
 int linkend=end;while(linkend>authority){if(suffix(buffer[linkend-1])){linkend--;continue;}if(linkend>=authority+2&&((buffer[linkend-1]=='*'&&buffer[linkend-2]=='*')||(buffer[linkend-1]=='~'&&buffer[linkend-2]=='~'))){linkend-=2;continue;}break;}
 out=clean(i,linkend,authority,out);for(int j=linkend;j<end;j++)buffer[out++]=buffer[j];i=end;
 }return out;
}
__attribute__((export_name("workshop_on_message"))) int on_message(int event, int length){(void)event;return transform(length);}
